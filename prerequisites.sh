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
    sudo apt-get install \
        libwayland-dev \
        libxkbcommon-dev \
        libudev-dev \
        libinput-dev \
        libgbm-dev \
        libseat-dev \
        libdbus-1-dev \
        libdisplay-info-dev \
        pipewire-dev \
        freetype-dev \
        fontconfig-dev \
        mesa-libEGL-dev \
        mesa-libGL-dev
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
