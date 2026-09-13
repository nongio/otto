# Checks that the package and the module assemble a working system — not that
# the desktop runs. Otto renders through KMS on a real GPU, which a plain
# QEMU machine does not have, so the compositor is never expected to come up
# here; everything around it is.
{ testers, ottoModule }:

testers.nixosTest {
  name = "otto-packaging";

  nodes.machine = { pkgs, ... }: {
    imports = [ ottoModule ./vm.nix ];
    # readelf, for the RUNPATH check below.
    environment.systemPackages = [ pkgs.binutils ];
  };

  testScript = ''
    machine.wait_for_unit("multi-user.target")

    with subtest("every binary resolves its shared libraries"):
        # libwayland is dlopen'd, so a missing rpath shows up as a panic at
        # runtime rather than a link error at build time.
        otto = machine.succeed("readlink -f $(command -v otto)").strip()
        store = otto.rsplit("/libexec/", 1)[0].rsplit("/bin/", 1)[0]
        # `.*-wrapped` too: wrapProgram leaves the real executable as a dotfile.
        missing = machine.succeed(
            f"for b in {store}/bin/* {store}/libexec/* {store}/libexec/.*-wrapped; do "
            "[ -f $b ] && ldd $b 2>/dev/null | grep 'not found' && echo $b; done; true"
        )
        assert missing.strip() == "", f"unresolved libraries:\n{missing}"
        # ldd lists only DT_NEEDED, so it never shows libwayland; what the
        # package guarantees is that its directory is on the RUNPATH.
        for b in ["libexec/otto", "bin/otto-bar"]:
            runpath = machine.succeed(f"readelf -d {store}/{b} | grep RUNPATH")
            wayland_dir = [d for d in runpath.split("[", 1)[1].rstrip("]\n").split(":") if "-wayland-" in d]
            assert wayland_dir, f"{b} has no wayland directory on its RUNPATH:\n{runpath}"
            machine.succeed(f"test -e {wayland_dir[0]}/libwayland-client.so.0")

    with subtest("the session entry points into the store"):
        entry = machine.succeed(
            "cat /run/current-system/sw/share/wayland-sessions/otto.desktop"
        )
        assert "/nix/store/" in entry, f"otto.desktop does not name a store path:\n{entry}"
        exec_line = [l for l in entry.splitlines() if l.startswith("Exec=")][0]
        machine.succeed(f"test -x {exec_line.split('=', 1)[1].split()[0]}")

    with subtest("otto-lock has a PAM stack of its own"):
        machine.succeed("test -f /etc/pam.d/otto-lock")
        machine.succeed("grep -q pam_unix /etc/pam.d/otto-lock")

    with subtest("the portal backend is registered and activatable"):
        machine.succeed(
            "test -f /run/current-system/sw/share/xdg-desktop-portal/portals/otto.portal"
        )
        service = machine.succeed(
            "cat /run/current-system/sw/share/dbus-1/services/"
            "org.freedesktop.impl.portal.desktop.otto.service"
        )
        assert "/libexec/xdg-desktop-portal-otto" in service, service
        exec_path = [l for l in service.splitlines() if l.startswith("Exec=")][0]
        machine.succeed(f"test -x {exec_path.split('=', 1)[1].strip()}")
        # Nobody is logged in, so there is no user manager to ask; check the
        # unit the D-Bus service activates is where systemd --user looks.
        unit = machine.succeed("readlink -f /etc/systemd/user/xdg-desktop-portal-otto.service").strip()
        assert unit.startswith("/nix/store/"), unit
        machine.succeed(f"grep -q '^ExecStart=/nix/store/.*/libexec/xdg-desktop-portal-otto$' {unit}")

    with subtest("the system configuration Otto's module asks for is in place"):
        # Enabled, not active: with no GPU the greeter it spawns cannot render.
        machine.succeed("systemctl is-enabled greetd.service")
        machine.succeed("systemctl is-active dbus.service")
        machine.succeed("ls /dev/dri/card*")
        machine.succeed("command -v Xwayland")
        machine.succeed("fc-list | grep -qi inter")

    with subtest("greetd launches the greeter from the store"):
        # The unit only names greetd's config; the session command lives there.
        unit = machine.succeed("systemctl cat greetd.service")
        config = unit.split("--config ", 1)[1].split()[0]
        greeter = machine.succeed(f"cat {config}")
        assert "/bin/otto --login" in greeter, greeter
        assert 'user = "greeter"' in greeter, greeter
        command = [l for l in greeter.splitlines() if l.startswith("command")][0]
        machine.succeed(f"test -x {command.split()[-2]}")
  '';
}
