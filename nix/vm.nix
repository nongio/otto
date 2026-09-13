# A NixOS machine with Otto as its desktop, for `nixosConfigurations.vm` and
# the packaging check in vm-test.nix.
#
# Otto renders through KMS on a real GPU. A plain QEMU machine has no such
# device, so this VM is not a way to *use* Otto — pass a GPU through to the
# guest for that. It exists to prove the package and the module assemble a
# system correctly: the session entry, the PAM stack, the portal, the units
# and the shared libraries.
{ pkgs, lib, ... }:

{
  programs.otto = {
    enable = true;
    greeter.enable = true;
  };

  users.users.alice = {
    isNormalUser = true;
    password = "alice";
    extraGroups = [ "wheel" "video" "input" ];
  };

  # virtio-gpu is the one QEMU display with a KMS driver, so the guest at
  # least has a DRM device for greetd to hand over.
  virtualisation = {
    memorySize = 3072;
    cores = 4;
    qemu.options = [ "-vga none" "-device virtio-gpu-pci" ];
  };

  networking.hostName = "otto-vm";
  system.stateVersion = "25.11";
}
