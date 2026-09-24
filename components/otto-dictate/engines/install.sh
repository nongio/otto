#!/bin/sh
# Set up dictation for the current user: build the engines, fetch the models,
# install otto-dictate and start it with the session.
#
#   install.sh             everything, Parakeet as the engine
#   install.sh whisper     everything, Whisper as the engine
#
# Needs git, cmake, a C++ compiler, the Vulkan headers, glslc and spirv-headers.
set -eu

engine=${1:-parakeet}
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}/otto-voice-poc
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
autostart=${XDG_CONFIG_HOME:-$HOME/.config}/autostart
hf=https://huggingface.co

mkdir -p "$data/models" "$units" "$autostart" "$HOME/.local/bin"

# whisper.cpp builds both servers; Parakeet's is ours, dropped into its examples.
[ -d "$data/whisper.cpp" ] || git clone https://github.com/ggml-org/whisper.cpp "$data/whisper.cpp"
mkdir -p "$data/whisper.cpp/examples/parakeet-server"
cp "$here/parakeet-server/"* "$data/whisper.cpp/examples/parakeet-server/"
grep -q parakeet-server "$data/whisper.cpp/examples/CMakeLists.txt" ||
    sed -i 's|    add_subdirectory(parakeet-cli)|&\n    add_subdirectory(parakeet-server)|' "$data/whisper.cpp/examples/CMakeLists.txt"
cmake -S "$data/whisper.cpp" -B "$data/whisper.cpp/build-vulkan" -DGGML_VULKAN=ON -DCMAKE_BUILD_TYPE=Release
cmake --build "$data/whisper.cpp/build-vulkan" -j"$(nproc)" --target whisper-server parakeet-server

fetch() {
    [ -s "$data/models/$2" ] || curl -fL --retry 3 -o "$data/models/$2" "$hf/$1/resolve/main/$2"
}
fetch ggml-org/parakeet-GGUF ggml-parakeet-tdt-0.6b-v3-f16.bin
fetch ggerganov/whisper.cpp ggml-small.en-q5_1.bin

cargo build --release --manifest-path "$repo/Cargo.toml" -p otto-dictate
install -m755 "$repo/target/release/otto-dictate" "$HOME/.local/bin/otto-dictate"
sed "s|^Exec=otto-dictate|Exec=$HOME/.local/bin/otto-dictate|" "$here/otto-dictate.desktop" > "$autostart/otto-dictate.desktop"

cp "$here/otto-stt-parakeet.service" "$here/otto-stt-whisper.service" "$units/"
systemctl --user daemon-reload
systemctl --user disable otto-stt-parakeet otto-stt-whisper 2>/dev/null || true
systemctl --user enable --now "otto-stt-$engine"
