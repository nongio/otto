# NixOS

Otto ships a Nix flake: a package with the compositor and every component,
and a NixOS module that installs it as a login session. Nothing here needs a
release tarball — the flake builds from the repository.

## Enabling the desktop

Add the flake as an input and import its module:

```nix
# flake.nix
{
  inputs.otto.url = "github:nongio/otto";

  outputs = { nixpkgs, otto, ... }: {
    nixosConfigurations.laptop = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        otto.nixosModules.otto
        ./configuration.nix
      ];
    };
  };
}
```

```nix
# configuration.nix
{
  programs.otto.enable = true;
}
```

`nixos-rebuild switch` and pick **Otto** in your display manager. The module
takes care of what the Arch, Debian and RPM packages leave to you:

- the session entry, so GDM, SDDM or greetd can start it;
- `/etc/otto/config.toml`, from the shipped example — the same file the other
  packages install, so the Dock is not empty on first login;
- the PAM service `otto-lock` authenticates against, with fingerprint unlock
  whenever `services.fprintd.enable` is on;
- the xdg-desktop-portal backend for screen sharing, screenshots and
  settings, with `xdg-desktop-portal-gtk` as the fallback;
- PipeWire with WirePlumber, graphics drivers, D-Bus, polkit, the Inter and
  Noto fonts the chrome asks for, and Xwayland for X11 applications.

## Options

| Option | Default | Effect |
|--------|---------|--------|
| `programs.otto.enable` | `false` | Install the desktop |
| `programs.otto.package` | the flake's `otto` | Which build to install |
| `programs.otto.config` | the shipped example | File installed as `/etc/otto/config.toml`; `null` leaves `/etc/otto` to you |
| `programs.otto.extraPackages` | `[ pkgs.foot ]` | Applications installed with the desktop — the example Dock pins a terminal |
| `programs.otto.greeter.enable` | `false` | Run greetd with `otto --login` as the login screen, see [Login Greeter](login-greeter.md) |
| `programs.otto.portal.enable` | `true` | Register the portal backend |

To configure the whole machine, point `programs.otto.config` at your own
file:

```nix
programs.otto.config = ./otto.toml;
```

Per-user settings go in `~/.config/otto/config.toml` as on any other
distribution — see [Configuration](configuration.md).

## The greeter

```nix
programs.otto.greeter.enable = true;
```

sets up greetd with Otto hosting its own greeter. Disable any other display
manager (`services.displayManager.gdm.enable`, `services.displayManager.sddm.enable`)
— only one may own the console.

## Checking the packaging

Otto renders through KMS on a real GPU, so it needs hardware — a laptop, a
desktop, or a virtual machine with a GPU passed through to it. A plain QEMU
machine will not run the desktop.

The flake still carries such a machine, to check that the package and the
module assemble a system correctly:

```sh
nix build .#checks.x86_64-linux.vm
```

boots it and verifies that every binary resolves its shared libraries, that
the session entry points into the store, that `otto-lock` has a PAM stack,
that the portal backend is registered and activatable, and that greetd
launches the greeter. It does not start the desktop.

```sh
nix build .#nixosConfigurations.vm.config.system.build.vm
./result/bin/run-otto-vm-vm
```

builds the same machine to poke at by hand.

## Building without the module

```sh
nix build github:nongio/otto      # ./result/bin/otto and the components
nix run github:nongio/otto -- --winit   # a window inside your current session
nix develop                       # a shell with the toolchain and libraries
```

## How the build works

Two things distinguish the flake from a plain `cargo build`:

- **Skia.** `skia-safe` downloads a prebuilt Skia for the exact feature set
  the workspace enables. The sandbox has no network, so `nix/package.nix`
  fetches that archive as a fixed-output derivation and hands it over as a
  `file://` URL. When the `skia-safe` version or its features change, the
  build fails naming the archive it wanted; update the key and hash there.
- **Git dependencies.** `smithay` and `lay-rs` come from git.
  `Cargo.lock` pins their revisions, and the package reads them from there,
  so there are no hashes to keep in sync.

The build runs the unit tests and the `headless_basic` integration suite.
The headless backend needs no GPU — it drives real Wayland clients against a
real compositor with no renderer — so it runs inside the sandbox. Suites that
need a PipeWire daemon or a session bus (`screenshare`, `rdp_bridge`) run in
CI instead.
