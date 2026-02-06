# Maintainer: Riccardo Canalicchio <riccardo.canalicchio@gmail.com>

pkgname=otto-bin
pkgver=0.14.0
pkgrel=1
pkgdesc="A visually-focused desktop system designed around smooth animations, thoughtful gestures and careful attention to detail, inspired by familiar macOS interactions."
url="https://github.com/nongio/otto"
license=("MIT")
arch=("x86_64")
provides=("otto")
conflicts=("otto")
depends=("libdrm" "systemd-libs" "mesa" "libxkbcommon" "wayland" "libinput" "dbus" "seatd" "pipewire" "freetype2" "fontconfig" "pixman" "noto-fonts")
optdepends=("xdg-desktop-portal: Desktop integration")
source=("https://github.com/nongio/otto/releases/download/v$pkgver/otto-$pkgver-x86_64.tar.gz")
sha256sums=("SKIP")

package() {
    cd "$srcdir"
    
    # Install binaries
    install -Dm755 target/release/otto "$pkgdir/usr/bin/otto"
    install -Dm755 target/release/xdg-desktop-portal-otto "$pkgdir/usr/libexec/xdg-desktop-portal-otto"
    
    # Install documentation
    install -Dm644 README.md "$pkgdir/usr/share/doc/otto/README.md"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 components/xdg-desktop-portal-otto/portals.conf.example "$pkgdir/usr/share/doc/otto/portals.conf.example"
    
    # Install configuration
    install -Dm644 otto_config.example.toml "$pkgdir/etc/otto/config.toml"
    
    # Install desktop files
    install -Dm644 resources/otto.desktop "$pkgdir/usr/share/wayland-sessions/otto.desktop"
    install -Dm644 components/xdg-desktop-portal-otto/otto.portal "$pkgdir/usr/share/xdg-desktop-portal/portals/otto.portal"
    install -Dm644 components/xdg-desktop-portal-otto/org.freedesktop.impl.portal.desktop.otto.service "$pkgdir/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.otto.service"
}
