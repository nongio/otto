# Maintainer: Riccardo Canalicchio <riccardo.canalicchio@gmail.com>

pkgname=otto-bin
pkgver=1.2.0
# Cargo's version (names the release tarball) and the git tag. They differ
# from pkgver for a prerelease: '-' is illegal in pkgver, and pacman sorts
# a '~' suffix *after* the plain version rather than before it.
_ver=1.2.0
_tag=v1.2.0
pkgrel=1
pkgdesc="A visually-focused desktop system designed around smooth animations, thoughtful gestures and careful attention to detail, inspired by familiar macOS interactions."
url="https://github.com/nongio/otto"
license=("MIT")
arch=("x86_64")
provides=("otto")
conflicts=("otto")
depends=("libdrm" "systemd-libs" "mesa" "libxkbcommon" "wayland" "libinput" "dbus" "seatd" "pipewire" "freetype2" "fontconfig" "pixman" "noto-fonts" "gstreamer" "gst-plugins-base-libs")
optdepends=("xdg-desktop-portal: Desktop integration" "fprintd: fingerprint unlock for otto-lock and otto-greeter" "greetd: login manager otto --login hosts a greeter for" "gst-plugin-pipewire: otto-rdp video capture" "gst-plugins-bad: otto-rdp hardware H.264 (VA-API)")
source=("https://github.com/nongio/otto/releases/download/$_tag/otto-$_ver-x86_64.tar.gz")
sha256sums=("SKIP")
# Files pacman must never clobber: a modified config becomes .pacnew on
# upgrade and .pacsave on removal, instead of being silently overwritten or
# deleted when swapping between the otto-bin/otto-git/otto-nightly-bin variants.
backup=("etc/otto/config.toml" "etc/pam.d/otto-lock")

package() {
    # The tarball directory is named after Cargo's version, which is not
    # pkgver: '-' is illegal in pkgver, so 1.0.0-rc.1 becomes 1.0.0rc1.
    cd "$srcdir/otto-$_ver"
    
    # Install binaries
    install -Dm755 target/release/otto "$pkgdir/usr/bin/otto"
    install -Dm755 target/release/otto-bar "$pkgdir/usr/bin/otto-bar"
    install -Dm755 target/release/otto-islands "$pkgdir/usr/bin/otto-islands"
    install -Dm755 target/release/otto-lock "$pkgdir/usr/bin/otto-lock"
    install -Dm755 target/release/otto-greeter "$pkgdir/usr/bin/otto-greeter"
    install -Dm755 target/release/otto-rdp "$pkgdir/usr/bin/otto-rdp"
    install -Dm755 target/release/otto-settings "$pkgdir/usr/bin/otto-settings"
    install -Dm755 target/release/otto-files "$pkgdir/usr/bin/otto-files"
    install -Dm755 target/release/otto-launcher "$pkgdir/usr/bin/otto-launcher"
    install -Dm755 target/release/otto-quickview "$pkgdir/usr/bin/otto-quickview"
    install -Dm755 target/release/xdg-desktop-portal-otto "$pkgdir/usr/libexec/xdg-desktop-portal-otto"
    
    # Install documentation
    install -Dm644 README.md "$pkgdir/usr/share/doc/otto/README.md"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 components/xdg-desktop-portal-otto/portals.conf.example "$pkgdir/usr/share/doc/otto/portals.conf.example"
    
    # Install configuration
    # Both the live config and the example it was copied from. config.toml is
    # in backup(), so an upgrade leaves a modified one alone and writes
    # .pacnew beside it; only removing the package (switching between the
    # otto-bin, otto-git and otto-nightly-bin variants) moves it to .pacsave.
    # Shipping it matters: with no config at all Otto falls back to
    # compiled-in defaults, which is an empty dock and no bookmarks.
    install -Dm644 otto_config.example.toml "$pkgdir/etc/otto/config.toml"
    install -Dm644 otto_config.example.toml "$pkgdir/etc/otto/config.example.toml"
    # PAM stack otto-lock authenticates against; without it PAM falls through
    # to `other`, which denies everything.
    install -Dm644 components/otto-lock/otto-lock.pam "$pkgdir/etc/pam.d/otto-lock"
    
    # Install desktop files
    install -Dm644 resources/otto.desktop "$pkgdir/usr/share/wayland-sessions/otto.desktop"
    install -Dm644 resources/otto-files.desktop "$pkgdir/usr/share/applications/otto-files.desktop"
    # The Trash window: the same binary behind its own entry, so it gets its
    # own icon in the dock and the applications list.
    install -Dm644 resources/otto-trash.desktop "$pkgdir/usr/share/applications/otto-trash.desktop"
    install -Dm644 resources/otto-settings.desktop "$pkgdir/usr/share/applications/otto-settings.desktop"

    # Install icons
    for _px in 16 24 32 48 64 128 256 512; do
        install -Dm644 "components/otto-files/resources/icons/hicolor/${_px}x${_px}/apps/otto-files.png" \
            "$pkgdir/usr/share/icons/hicolor/${_px}x${_px}/apps/otto-files.png"
    done
    install -Dm644 components/otto-files/resources/icons/hicolor/scalable/apps/otto-files.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/otto-files.svg"
    install -Dm644 components/xdg-desktop-portal-otto/otto.portal "$pkgdir/usr/share/xdg-desktop-portal/portals/otto.portal"
    install -Dm644 components/xdg-desktop-portal-otto/org.freedesktop.impl.portal.desktop.otto.service "$pkgdir/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.otto.service"
    # The v1.0.0-rc1 tarball shipped without this unit, and the D-Bus service
    # file above names it in SystemdService=. Synthesise it when it is absent
    # so the portal still activates; drop the fallback once every supported
    # release tarball carries the file.
    _unit=components/xdg-desktop-portal-otto/xdg-desktop-portal-otto.service
    if [ ! -f "$_unit" ]; then
        _unit="$srcdir/xdg-desktop-portal-otto.service"
        cat > "$_unit" <<'UNIT'
[Unit]
Description=Portal service (Otto implementation)
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=dbus
BusName=org.freedesktop.impl.portal.desktop.otto
ExecStart=/usr/libexec/xdg-desktop-portal-otto
Restart=on-failure
UNIT
    fi
    install -Dm644 "$_unit" "$pkgdir/usr/lib/systemd/user/xdg-desktop-portal-otto.service"
}
