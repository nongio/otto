// A whisper-server stand-in for Parakeet: POST /inference with a 16 kHz mono
// 16-bit WAV in the "file" field answers WebVTT with one cue per word.
#include "parakeet.h"
#include "ggml-backend.h"
#include "httplib.h"

#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

static bool wav_pcm16(const std::string & data, std::vector<float> & out) {
    if (data.size() < 12 || data.compare(0, 4, "RIFF") != 0 || data.compare(8, 4, "WAVE") != 0) return false;
    size_t at = 12;
    while (at + 8 <= data.size()) {
        uint32_t len; memcpy(&len, data.data() + at + 4, 4);
        if (data.compare(at, 4, "data") == 0) {
            size_t n = std::min<size_t>(len, data.size() - at - 8) / 2;
            out.resize(n);
            const char * p = data.data() + at + 8;
            for (size_t i = 0; i < n; i++) { int16_t s; memcpy(&s, p + 2 * i, 2); out[i] = s / 32768.0f; }
            return true;
        }
        at += 8 + len + (len & 1);
    }
    return false;
}

static std::string stamp(int64_t cs) {
    char buf[32];
    snprintf(buf, sizeof buf, "%02d:%02d:%02d.%03d", (int)(cs / 360000), (int)(cs / 6000 % 60), (int)(cs / 100 % 60), (int)(cs % 100 * 10));
    return buf;
}

int main(int argc, char ** argv) {
    std::string model = "models/ggml-parakeet-tdt-0.6b-v3-f16.bin", host = "127.0.0.1";
    int port = 8080, threads = 4;
    for (int i = 1; i + 1 < argc; i += 2) {
        std::string a = argv[i];
        if (a == "-m") model = argv[i + 1];
        else if (a == "--host") host = argv[i + 1];
        else if (a == "--port") port = std::stoi(argv[i + 1]);
        else if (a == "-t") threads = std::stoi(argv[i + 1]);
    }
    ggml_backend_load_all();
    parakeet_context * ctx = parakeet_init_from_file_with_params(model.c_str(), parakeet_context_default_params());
    if (!ctx) { fprintf(stderr, "cannot load %s\n", model.c_str()); return 1; }
    std::mutex lock;

    httplib::Server server;
    server.Get("/", [](const httplib::Request &, httplib::Response & res) { res.set_content("parakeet-server", "text/plain"); });
    server.Post("/inference", [&](const httplib::Request & req, httplib::Response & res) {
        std::vector<float> pcm;
        if (!req.has_file("file") || !wav_pcm16(req.get_file_value("file").content, pcm)) {
            res.status = 400; res.set_content("expected a 16-bit PCM WAV in \"file\"", "text/plain"); return;
        }
        std::lock_guard<std::mutex> guard(lock);
        std::string vtt = "WEBVTT\n\n";
        if (pcm.size() >= 1600) {
            parakeet_full_params params = parakeet_full_default_params(PARAKEET_SAMPLING_GREEDY);
            params.n_threads = threads;
            if (parakeet_full(ctx, params, pcm.data(), pcm.size()) != 0) { res.status = 500; return; }
            // One cue per word: its tokens' text, from the first token's start
            // to the last token's end (centiseconds).
            std::string word; int64_t t0 = 0, t1 = 0;
            auto flush = [&]() {
                if (!word.empty()) vtt += stamp(t0) + " --> " + stamp(std::max(t1, t0)) + "\n" + word + "\n\n";
                word.clear();
            };
            for (int s = 0; s < parakeet_full_n_segments(ctx); s++) {
                for (int j = 0; j < parakeet_full_n_tokens(ctx, s); j++) {
                    parakeet_token_data td = parakeet_full_get_token_data(ctx, s, j);
                    char buf[256];
                    parakeet_token_to_text(parakeet_token_to_str(ctx, td.id), true, buf, sizeof buf);
                    if (td.is_word_start) { flush(); t0 = td.t0; }
                    word += buf; t1 = td.t1;
                }
            }
            flush();
        }
        res.set_content(vtt, "text/vtt");
    });
    fprintf(stderr, "listening on %s:%d\n", host.c_str(), port);
    server.listen(host, port);
    parakeet_free(ctx);
}
