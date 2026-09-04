#!/usr/bin/env bash
# Assert that an installed Otto package put everything where the desktop
# expects to find it. Run *inside* the container, after the package manager
# has installed the package — the point is to check the installed filesystem,
# not the archive, so a mis-declared destination or a missing asset fails here
# rather than on a user's machine.
#
#   verify-install.sh deb    # Debian/Ubuntu layout
#   verify-install.sh rpm    # Fedora/RHEL layout
#   verify-install.sh arch   # Arch layout (otto-bin / otto-git / nightly)
#
# The flavours genuinely differ, and the differences are deliberate:
#   - the deb and the Arch package ship otto-files' raster icons, the rpm
#     only the SVG
#   - the deb ships otto-lock.pam as an example under /usr/share/doc, because
#     Debian's stack is `common-auth`; the rpm and the Arch package install it
#     to /etc/pam.d, because their stack is the `system-auth` it includes
#   - Arch puts the licence under /usr/share/licenses/$pkgname, not
#     /usr/share/doc/otto
set -uo pipefail

flavour="${1:?usage: verify-install.sh deb|rpm|arch}"
fail=0

check() {
    local path="$1" kind="${2:-file}"
    case "$kind" in
        exec) [[ -x "$path" ]] || { echo "MISSING or not executable: $path"; fail=1; return; } ;;
        *)    [[ -f "$path" ]] || { echo "MISSING: $path"; fail=1; return; } ;;
    esac
    echo "  ok  $path"
}

echo "== binaries =="
for b in otto otto-bar otto-islands otto-lock otto-settings otto-files \
         otto-launcher otto-quickview otto-greeter otto-rdp; do
    check "/usr/bin/$b" exec
done
check /usr/libexec/xdg-desktop-portal-otto exec

echo "== session and applications =="
check /usr/share/wayland-sessions/otto.desktop
check /usr/share/applications/otto-files.desktop
check /usr/share/applications/otto-settings.desktop
# The Trash window is otto-files behind its own entry, so it gets its own
# icon in the dock and the applications list. Every package must ship it.
check /usr/share/applications/otto-trash.desktop

echo "== portal backend =="
check /usr/share/xdg-desktop-portal/portals/otto.portal
check /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.otto.service
# The D-Bus service file names this unit in SystemdService=; shipping one
# without the other leaves the portal bus-activatable but unstartable.
check /usr/lib/systemd/user/xdg-desktop-portal-otto.service

echo "== configuration =="
check /etc/otto/config.toml

echo "== documentation =="
check /usr/share/doc/otto/README.md
check /usr/share/doc/otto/portals.conf.example
if [[ "$flavour" == arch ]]; then
    # Arch's convention; $pkgname varies across the three variants.
    ls /usr/share/licenses/otto*/LICENSE >/dev/null 2>&1 \
        && echo "  ok  /usr/share/licenses/otto*/LICENSE" \
        || { echo "MISSING: /usr/share/licenses/otto*/LICENSE"; fail=1; }
else
    check /usr/share/doc/otto/LICENSE
fi

echo "== icons =="
check /usr/share/icons/hicolor/scalable/apps/otto-files.svg
if [[ "$flavour" != rpm ]]; then
    for px in 16 24 32 48 64 128 256 512; do
        check "/usr/share/icons/hicolor/${px}x${px}/apps/otto-files.png"
    done
fi

echo "== PAM =="
if [[ "$flavour" == deb ]]; then
    check /usr/share/doc/otto/otto-lock.pam.example
else
    check /etc/pam.d/otto-lock
fi

echo "== desktop entry is valid =="
if command -v desktop-file-validate >/dev/null; then
    for d in /usr/share/applications/otto-files.desktop \
             /usr/share/applications/otto-trash.desktop \
             /usr/share/applications/otto-settings.desktop; do
        [[ -f "$d" ]] || continue   # already reported missing above
        desktop-file-validate "$d" && echo "  ok  $d" || fail=1
    done
    # The session entry is validated separately. desktop-file-validate checks
    # against the *application* entry spec, where `DesktopNames` is not a
    # registered key — but it is exactly what a Wayland session entry is
    # supposed to carry, and what session managers read (GNOME's and KDE's
    # session files carry it too). Treat that one complaint as expected and
    # fail on anything else.
    session=/usr/share/wayland-sessions/otto.desktop
    if [[ -f "$session" ]]; then
        out=$(desktop-file-validate "$session" 2>&1 | grep -v 'key "DesktopNames"')
        if [[ -n "$out" ]]; then
            echo "$out"; fail=1
        else
            echo "  ok  $session"
        fi
    fi
else
    echo "  (desktop-file-validate not installed, skipped)"
fi

echo "== Exec= targets resolve =="
# A session entry pointing at a binary the package does not ship is the
# classic packaging break: the session shows up in the greeter and dies.
for d in /usr/share/wayland-sessions/otto.desktop \
         /usr/share/applications/otto-files.desktop \
         /usr/share/applications/otto-trash.desktop \
         /usr/share/applications/otto-settings.desktop; do
    [[ -f "$d" ]] || continue   # already reported missing above
    exe=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$d" | head -1)
    [[ -n "$exe" ]] || { echo "no Exec= in $d"; fail=1; continue; }
    if [[ "$exe" == /* ]]; then
        [[ -x "$exe" ]] || { echo "Exec= target missing: $exe (from $d)"; fail=1; continue; }
    else
        command -v "$exe" >/dev/null || { echo "Exec= target not on PATH: $exe (from $d)"; fail=1; continue; }
    fi
    echo "  ok  $d -> $exe"
done

echo "== shared libraries resolve =="
# The real dependency test. A package can declare every dependency it likes;
# what matters is whether the dynamic loader finds them on a machine that has
# installed nothing but this package. Any "not found" here is a dependency the
# package failed to declare.
#
# Only meaningful for a CI-built package. The deb's dependency list is
# `$auto` plus an explicit tail, and `$auto` is dpkg-shlibdeps' work — build
# the deb anywhere without dpkg-shlibdeps (an Arch workstation, say) and it
# contributes nothing, so half the libraries go undeclared through no fault
# of the packaging. OTTO_SKIP_RUN marks that case; layout is still checked.
if [[ "${OTTO_SKIP_RUN:-0}" == 1 ]]; then
    echo "  (skipped: locally built package, dependency list is not authoritative)"
else
for b in /usr/bin/otto /usr/bin/otto-bar /usr/bin/otto-islands /usr/bin/otto-lock \
         /usr/bin/otto-settings /usr/bin/otto-files /usr/bin/otto-launcher \
         /usr/bin/otto-quickview /usr/bin/otto-greeter /usr/bin/otto-rdp \
         /usr/libexec/xdg-desktop-portal-otto; do
    [[ -x "$b" ]] || continue   # already reported missing above
    # `version \`GLIBC_2.44' not found` is the foreign-glibc case above, not a
    # missing dependency — the library was found, it is just older than the
    # build machine's. Drop those lines when runs are being skipped.
    missing=$(ldd "$b" 2>/dev/null | grep 'not found' \
        | { [[ "${OTTO_SKIP_RUN:-0}" == 1 ]] && grep -v 'GLIBC_' || cat; } \
        | awk '{print $1}' | sort -u)
    if [[ -n "$missing" ]]; then
        echo "UNRESOLVED in $b:"
        echo "$missing" | sed 's/^/      /'
        fail=1
    else
        echo "  ok  $b"
    fi
done
fi

echo "== binaries run =="
# A locally built package carries binaries linked against the build machine's
# glibc, which is newer than the target distribution's — they cannot run here,
# and that says nothing about the packaging under test. OTTO_SKIP_RUN=1 (set
# by test-installers.sh for OTTO_LOCAL=1) checks layout and declared
# dependencies only. The release packages always run this section.
if [[ "${OTTO_SKIP_RUN:-0}" == 1 ]]; then
    echo "  (skipped: locally built binaries, foreign glibc)"
else
# --version loads the binary and every library it links, then exits: enough
# to prove the install is runnable without a seat, a GPU or a compositor.
"/usr/bin/otto" --version || { echo "otto --version failed"; fail=1; }
for b in otto-bar otto-islands otto-lock otto-settings otto-files \
         otto-launcher otto-quickview otto-greeter otto-rdp; do
    [[ -x "/usr/bin/$b" ]] || continue
    # Not every component parses --version; a component that instead prints
    # usage and exits non-zero has still loaded successfully. Only a loader
    # failure (127, or a message from ld.so) is a real failure.
    out=$("/usr/bin/$b" --version 2>&1); rc=$?
    if grep -qiE 'error while loading shared libraries|cannot open shared object' <<<"$out"; then
        echo "LOADER FAILURE: $b: $out"; fail=1
    elif (( rc == 127 )); then
        echo "NOT EXECUTABLE: $b (exit 127): $out"; fail=1
    else
        echo "  ok  $b (exit $rc)"
    fi
done
fi

if (( fail )); then
    echo "FAILED: $flavour install is incomplete"
    exit 1
fi
echo "PASS: $flavour install complete, linked and runnable"
