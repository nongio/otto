fn main() {
    // Compile the FairPlay/playfair C library (only for airplay feature)
    #[cfg(feature = "airplay")]
    {
        cc::Build::new()
            .file("csrc/playfair/playfair.c")
            .file("csrc/playfair/omg_hax.c")
            .file("csrc/playfair/hand_garble.c")
            .file("csrc/playfair/modified_md5.c")
            .file("csrc/playfair/sap_hash.c")
            .file("csrc/fairplay_sender.c")
            .include("csrc")
            .warnings(false)
            .compile("fairplay");

        println!("cargo:rerun-if-changed=csrc/");
    }
}
