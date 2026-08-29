#!/bin/bash

# Detect Linux distribution
if [ -f /etc/os-release ]; then
    . /etc/os-release
    DISTRO=$ID
    DISTRO_LIKE=$ID_LIKE
else
    echo "Cannot detect Linux distribution. /etc/os-release not found."
    exit 1
fi

echo "Detected distribution: $DISTRO (like $DISTRO_LIKE)"

install_debian() {
    echo "Installing dependencies for Debian/Ubuntu based system..."
    sudo apt-get update
    sudo apt-get install -y \
        build-essential \
        pkg-config \
        clang \
        libclang-dev \
        libwayland-dev \
        libxkbcommon-dev \
        libudev-dev \
        libinput-dev \
        libgbm-dev \
        libseat-dev \
        libsystemd-dev \
        libdbus-1-dev \
        libpipewire-0.3-dev \
        libfreetype-dev \
        libfontconfig-dev \
        libegl1-mesa-dev \
        libgl1-mesa-dev \
        libgles2-mesa-dev \
        libxcb1-dev \
        libpixman-1-dev

    # libspa (pipewire crate 0.9) needs PipeWire >= 1.0 headers; Ubuntu 22.04
    # ships 0.3.48, whose spa_video_info_raw.modifier is int64_t and fails to
    # compile. Ubuntu 24.04+ and Debian 13+ are new enough. Say so plainly
    # rather than reaching for a third-party repository on the user's behalf.
    pw_version=$(pkg-config --modversion libpipewire-0.3 2>/dev/null)
    if [ -n "$pw_version" ] && [ "$(printf '%s\n1.0.0\n' "$pw_version" | sort -V | head -1)" != "1.0.0" ]; then
        echo "ERROR: PipeWire $pw_version is too old to build Otto (need >= 1.0)." >&2
        echo "Upgrade to a release that ships PipeWire 1.0 or later (Ubuntu 24.04+," >&2
        echo "Debian 13+), or install PipeWire >= 1.0 development headers yourself." >&2
        echo "On Ubuntu 22.04 the pipewire-debian/pipewire-upstream PPA has them." >&2
        exit 1
    fi

    # libdisplay-info-dev only exists from Ubuntu 24.04 / Debian 13 onwards.
    # On older releases, build it from source (smithay needs >= 0.1, < 0.4).
    if apt-cache show libdisplay-info-dev >/dev/null 2>&1; then
        sudo apt-get install -y libdisplay-info-dev
    else
        echo "libdisplay-info-dev not in repos; building libdisplay-info from source..."
        sudo apt-get install -y meson ninja-build hwdata git
        tmp=$(mktemp -d)
        git clone --depth 1 --branch 0.2.0 \
            https://gitlab.freedesktop.org/emersion/libdisplay-info.git "$tmp/libdisplay-info"
        meson setup "$tmp/libdisplay-info/build" "$tmp/libdisplay-info" \
            --prefix=/usr/local --buildtype=release
        ninja -C "$tmp/libdisplay-info/build"
        sudo ninja -C "$tmp/libdisplay-info/build" install
        sudo ldconfig
        rm -rf "$tmp"
    fi
}

install_redhat() {
    echo "Installing dependencies for RHEL/Fedora based system..."
    sudo dnf install \
        wayland-devel \
        libxkbcommon-devel \
        systemd-devel \
        libinput-devel \
        mesa-libgbm-devel \
        libseat-devel \
        dbus-devel \
        libdisplay-info-devel \
        pipewire-devel \
        freetype-devel \
        fontconfig-devel \
        mesa-libEGL-devel \
        mesa-libGL-devel
}

install_arch() {
    echo "Installing dependencies for Arch based system..."
    sudo pacman -S \
        wayland \
        libxkbcommon \
        systemd \
        libinput \
        mesa \
        libseat \
        dbus \
        libdisplay-info \
        pipewire \
        freetype \
        fontconfig \
        mesa-libEGL \
        mesa-libGL
}

case "$DISTRO" in
    ubuntu|debian|pop|mint|kali)
        install_debian
        ;;
    fedora|rhel|centos|rocky|almalinux)
        install_redhat
        ;;
    arch|manjaro)
        install_arch
        ;;
    *)
        # Check ID_LIKE if specific ID didn't match
        if [[ "$DISTRO_LIKE" == *"debian"* ]]; then
            install_debian
        elif [[ "$DISTRO_LIKE" == *"fedora"* ]] || [[ "$DISTRO_LIKE" == *"rhel"* ]]; then
            install_redhat
        else
            echo "Unsupported distribution: $DISTRO"
            echo "Please install the following packages manually:"
            echo "- libwayland"
            echo "- libxkbcommon"
            echo "- libudev"
            echo "- libinput"
            echo "- libgbm"
            echo "- libseat"
            echo "- dbus"
            echo "- libdisplay-info"
            echo "- pipewire"
            exit 1
        fi
        ;;
esac

echo "Dependencies installed successfully."
