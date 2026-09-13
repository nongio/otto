# Otto as a Nix package: the compositor and every component the Arch, Debian
# and RPM packages ship (see PKGBUILD for the canonical list), installed under
# one store path. The NixOS module in module.nix wires it into a system.
{ lib
, src
, stdenv
, rustPlatform
, fetchurl
, pkg-config
, makeWrapper
, wayland-scanner
, curl
, libdrm
, udev
, libgbm
, libglvnd
, libxkbcommon
, wayland
, libinput
, dbus
, seatd
, pipewire
, wireplumber
, freetype
, fontconfig
, pixman
, libdisplay-info
, fetchFromGitLab
, gst_all_1
, pam
, libx11
, xwayland
, grim
, wl-clipboard
, xdg-utils
, glibc
, shared-mime-info
}:

let
  cargoToml = lib.importTOML (src + "/Cargo.toml");

  # skia-safe's build script downloads a prebuilt Skia for the exact feature
  # set the workspace enables; the sandbox has no network, so it is fetched
  # here and handed over as a file:// URL. If the skia-safe version or its
  # features change, the build fails naming the archive it wanted — put the
  # new key and hash here. `cargo tree -e features -i skia-bindings --workspace`
  # shows the features, and the key is printed by the failing build.
  skiaTag = "0.93.1";
  skiaKey = "319323662b1685a112f5-x86_64-unknown-linux-gnu-egl-gl-jpegd-jpege-pdf-skottie-svg-textlayout-vulkan-wayland-webpd-webpe-x11";
  skiaBinaries = fetchurl {
    url = "https://github.com/rust-skia/skia-binaries/releases/download/${skiaTag}/skia-binaries-${skiaKey}.tar.gz";
    hash = "sha256-nIA0yLFpThRsMt8rvIkDsvVBnh0EbowfF1KD7W0MDI8=";
  };

  # The libdisplay-info crate binds the 0.1–0.3 ABI and refuses 0.4, which
  # is what nixpkgs carries; build 0.3.0 from the same recipe. Drop this
  # once Cargo.lock moves to a libdisplay-info-sys that accepts 0.4.
  libdisplay-info_0_3 = libdisplay-info.overrideAttrs (old: rec {
    version = "0.3.0";
    src = fetchFromGitLab {
      domain = "gitlab.freedesktop.org";
      owner = "emersion";
      repo = "libdisplay-info";
      tag = version;
      hash = "sha256-nXf2KGovNKvcchlHlzKBkAOeySMJXgxMpbi5z9gLrdc=";
    };
  });

  # Everything the distribution packages install. Built in one cargo
  # invocation so features unify once — and so one Skia key covers them all.
  members = [
    "otto"
    "otto-bar"
    "otto-islands"
    "otto-lock"
    "otto-greeter"
    "xdg-desktop-portal-otto"
    "otto-rdp"
    "otto-settings"
    "otto-files"
    "otto-launcher"
    "otto-emoji"
    "otto-quickview"
    "otto-media-kit"
  ];

  # GStreamer plugins otto-rdp and Quick View's playback worker load at
  # runtime: pipewiresrc, the VA-API H.264 encoder, the playbin demuxers and
  # decoders. The same set the .deb recommends.
  gstPlugins = with gst_all_1; [
    gstreamer
    gst-plugins-base
    gst-plugins-good
    gst-plugins-bad
    gst-libav
    pipewire
  ];
in
rustPlatform.buildRustPackage {
  pname = "otto";
  version = cargoToml.workspace.package.version;

  inherit src;

  cargoLock = {
    lockFile = src + "/Cargo.lock";
    # smithay and lay-rs come from git; Cargo.lock pins their revisions.
    allowBuiltinFetchGit = true;
  };

  nativeBuildInputs = [
    pkg-config
    makeWrapper
    wayland-scanner
    rustPlatform.bindgenHook
  ];

  buildInputs = [
    libdrm
    udev
    libgbm
    libglvnd
    libxkbcommon
    wayland
    libinput
    dbus
    seatd
    pipewire
    freetype
    fontconfig
    pixman
    libdisplay-info_0_3
    pam
    libx11
  ] ++ gstPlugins;

  SKIA_BINARIES_URL = "file://${skiaBinaries}";

  cargoBuildFlags = lib.concatMap (m: [ "-p" m ]) members;

  # The same tests CI runs, minus the ones that need something the sandbox
  # does not have: a session bus, a PipeWire daemon, the host's fonts. The
  # headless backend has no renderer at all — it drives real Wayland clients
  # against a real compositor with no GPU — so `headless_basic` is the one
  # integration suite that belongs here. `--features headless` adds a feature
  # to the compositor, so the check recompiles `otto`; the components are
  # unaffected.
  doCheck = true;

  # otto-files resolves MIME types through the shared MIME database.
  nativeCheckInputs = [ shared-mime-info ];

  preCheck = ''
    # The compositor opens its socket under XDG_RUNTIME_DIR, and otto-files
    # writes its thumbnail cache under HOME.
    export XDG_RUNTIME_DIR="$(mktemp -d)"
    export HOME="$(mktemp -d)"
    export XDG_DATA_DIRS="${shared-mime-info}/share''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
  '';

  checkPhase = ''
    runHook preCheck

    # The sandbox's build user cannot keep setgid on a file whose group it
    # does not belong to, so the special-bits test cannot pass here.
    cargo test --offline --profile $cargoCheckType --lib --workspace -- \
      --skip model::chmod_tests::special_bits_survive_a_toggle
    cargo test --offline --profile $cargoCheckType --features headless --test headless_basic

    runHook postCheck
  '';

  postInstall = ''
    # The portal backend lives in libexec, where its D-Bus service expects it.
    mkdir -p $out/libexec
    mv $out/bin/xdg-desktop-portal-otto $out/libexec/

    install -Dm644 README.md $out/share/doc/otto/README.md
    install -Dm644 LICENSE $out/share/licenses/otto/LICENSE
    install -Dm644 components/xdg-desktop-portal-otto/portals.conf.example $out/share/doc/otto/portals.conf.example
    install -Dm644 otto_config.example.toml $out/share/doc/otto/config.example.toml
    install -Dm644 components/otto-lock/otto-lock.pam $out/share/doc/otto/otto-lock.pam.example

    install -Dm644 resources/otto.desktop $out/share/wayland-sessions/otto.desktop
    install -Dm644 resources/otto-files.desktop $out/share/applications/otto-files.desktop
    install -Dm644 resources/otto-trash.desktop $out/share/applications/otto-trash.desktop
    install -Dm644 resources/otto-settings.desktop $out/share/applications/otto-settings.desktop
    substituteInPlace $out/share/wayland-sessions/otto.desktop \
      --replace-fail /usr/bin/otto $out/bin/otto

    for px in 16 24 32 48 64 128 256 512; do
      install -Dm644 components/otto-files/resources/icons/hicolor/''${px}x''${px}/apps/otto-files.png \
        $out/share/icons/hicolor/''${px}x''${px}/apps/otto-files.png
    done
    install -Dm644 components/otto-files/resources/icons/hicolor/scalable/apps/otto-files.svg \
      $out/share/icons/hicolor/scalable/apps/otto-files.svg

    install -Dm644 components/xdg-desktop-portal-otto/otto.portal $out/share/xdg-desktop-portal/portals/otto.portal
    install -Dm644 components/xdg-desktop-portal-otto/org.freedesktop.impl.portal.desktop.otto.service \
      $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.otto.service
    install -Dm644 components/xdg-desktop-portal-otto/xdg-desktop-portal-otto.service \
      $out/lib/systemd/user/xdg-desktop-portal-otto.service
    substituteInPlace \
      $out/share/dbus-1/services/org.freedesktop.impl.portal.desktop.otto.service \
      $out/lib/systemd/user/xdg-desktop-portal-otto.service \
      --replace-fail /usr/libexec/xdg-desktop-portal-otto $out/libexec/xdg-desktop-portal-otto
  '';

  postFixup = ''
    # libwayland is opened with dlopen (wayland-sys' `dlopen` feature, which
    # smithay's `use_system_lib` turns on), so nothing records a DT_NEEDED for
    # it and fixupPhase shrinks it back out of the rpath. Without this the
    # compositor panics with `NoWaylandLib` in `Display::new()` — and every
    # component panics the same way on `Connection::connect_to_env()`.
    for bin in $out/bin/* $out/libexec/*; do
      patchelf --add-rpath "${lib.makeLibraryPath [ wayland ]}" "$bin"
    done

    # Everything the desktop spawns by name rather than by path: its own
    # components, Xwayland, wpctl, grim (screenshots), wl-copy (the launcher's
    # calculator), xdg-open (Files and Settings), fc-list and `locale -a`
    # (Settings' font and language lists).
    #
    # `wrapProgram` in place would leave the real executable called
    # `.otto-wrapped`, which is the name `ps`, `pgrep` and `pkill` then see.
    # Wrapping out of libexec keeps the process called `otto`.
    mv $out/bin/otto $out/libexec/otto
    makeWrapper $out/libexec/otto $out/bin/otto \
      --prefix PATH : "$out/bin:${lib.makeBinPath [
        xwayland
        wireplumber
        grim
        wl-clipboard
        xdg-utils
        (lib.getBin fontconfig)
        (lib.getBin glibc)
      ]}"

    # The portal backend is D-Bus activated, so it does not inherit otto's PATH.
    wrapProgram $out/libexec/xdg-desktop-portal-otto \
      --prefix PATH : "${lib.makeBinPath [ grim ]}"

    for bin in otto-rdp otto-media-worker; do
      mv $out/bin/$bin $out/libexec/$bin
      makeWrapper $out/libexec/$bin $out/bin/$bin \
        --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" gstPlugins}"
    done
  '';

  passthru = {
    providedSessions = [ "otto" ];
    inherit skiaBinaries;
  };

  meta = {
    description = cargoToml.package.description;
    homepage = "https://github.com/nongio/otto";
    license = lib.licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "otto";
  };
}
