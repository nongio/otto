# Network

Otto shows and changes your network through NetworkManager. You join
networks from the indicator in the top bar, and manage the ones you have
saved in **Settings → Network**.

When NetworkManager needs a Wi-Fi password, Otto asks for it in its own
dialog, so Wi-Fi needs no other applet. Otto does not answer other
credentials: a VPN or an enterprise Wi-Fi network that needs a password to
connect needs it saved for all users (see
[Enterprise Wi-Fi and VPNs](#enterprise-wi-fi-and-vpns)), or another secret
agent such as `nm-applet` running alongside.

## The indicator

The indicator sits in the top bar between the keyboard layout and the
battery.

| It shows | When |
|----------|------|
| A fan with one to three bars lit | Connected over Wi-Fi; the bars are the signal |
| Three linked boxes | Connected by cable |
| The bars filling one after another | A connection is coming up |
| An empty fan | Nothing is connected |
| An empty fan, crossed out | Wi-Fi is off |
| Dimmed, with an exclamation mark | Connected, but the internet cannot be reached, for example behind a hotel's sign-in page |

It needs NetworkManager and a Wi-Fi or wired device. **Settings → Top bar →
Show network status in the top bar** turns it off, as does this in the
configuration file:

```toml
# ~/.config/otto/config.toml
[topbar]
show_network = false
```

## The menu

Click the indicator for the network menu:

- **Wi-Fi** turns the radio on and off.
  - **Airplane mode:** after the airplane key, or another program, turned the radio off, the menu says *Off with airplane mode*. Choosing **Wi-Fi** turns the radio back on.
  - **Wireless switch:** when the computer's own wireless switch is off, the menu says so and nothing on screen can turn Wi-Fi on. Flip the switch.

  The indicator follows either one as soon as it happens, and catches up as soon as the laptop wakes from sleep.
- The networks in range. The one you are on is ticked, with a line under it
  giving its band, channel, link rate and address. Saved networks come first,
  and the rest wait under **Other Networks**. A network that needs a password
  says **Secured**.
- **Join Other Network…** joins a network that does not broadcast its name.
- Your wired connections, whether a cable is plugged in, and the link speed.
- Your VPN and WireGuard connections, ticked while they are up. Pick one to
  start or stop it.
- **Disconnect from** the current network, and **Network Settings…**.

Opening the menu starts a scan, and the list updates while the menu is open.

## Joining a secured network

Picking a secured network you have not joined before opens the password
dialog. **Join** stays dimmed until the password could be right for that kind
of network: 8 to 63 characters for WPA2, any length for WPA3, and 5 or 13
characters (or 10 or 26 hex digits) for WEP. **Show password** reveals what
you typed. NetworkManager saves the password with the network, so it connects
again by itself.

The same dialog comes up when NetworkManager needs a password it does not
have, for example after the network's password was changed. Then it says that
the password was not accepted.

For a hidden network, **Join Other Network…** asks for the network's name and
how it is secured (**None**, **WPA2**, **WPA3** or **WEP**) as well.

### Networks saved by another applet

Some applets keep Wi-Fi passwords in your keyring rather than with
NetworkManager, and those networks cannot connect by themselves at login
without that applet. The first time you pick one from the menu, Otto asks
for its password and then has NetworkManager keep it with the network. From
then on it connects by itself, like any network joined in Otto. A network
set to ask for its password every time keeps asking.

### Enterprise Wi-Fi and VPNs

WPA Enterprise networks (a user name and a certificate, as at universities)
cannot be joined from the menu yet. Set them up with `nm-connection-editor`
and choose **Store the password for all users** from the icon in the
password field. After that they connect from the menu. The same goes
for a VPN that asks for a password: with it stored for all users, the switch
in the menu and in Settings is all it needs. A one-time code or token cannot
be stored, so for those keep `nm-applet` or another secret agent running.

## Settings → Network

The pane has a tab for **Wi-Fi** and one for **Ethernet** when the machine has
those, and one for **VPN**. It opens on the kind of connection you are using.

### Wi-Fi

The switch and the network in use. While you are connected, **Connection**
shows:

- **Signal** in bars, in words and as a percentage
- **Band** and channel
- **Link rate**
- **Security**
- **Access point**, the BSSID of the one you are associated with
- **Hardware address**

**Known networks** lists every saved network. Untick one to stop joining it
automatically, or **Forget** it.

### Ethernet

Each wired port's state, link speed and hardware address.

### TCP/IP and DNS

The Wi-Fi and Ethernet tabs both have **TCP/IP** and **DNS** for their
connection. On the Wi-Fi tab, **Settings for** picks which saved network you
are editing.

| Setting | Choices |
|---------|---------|
| **Configure IPv4** | Automatic (DHCP), Manually, Link-local only, Share this connection, Off |
| **Configure IPv6** | Automatic, Automatic DHCP only, Manually, Link-local only, Share this connection, Off |

**Manually** shows the address fields:

- **IPv4 address**, like `192.168.1.20/24`. Several are separated by commas. Without a prefix, /24 is assumed (/64 for IPv6).
- **Router**

Any other choice shows the addresses and router the connection has now.

**Share this connection** makes this machine the router for whatever is
plugged into that port, handing out addresses and passing traffic on through
your other connection.

DNS:

- **Automatic DNS**: also ask the servers the network offers. Turn it off to ask only yours.
- **DNS servers**: asked first. IPv4 and IPv6 addresses go in the same field, separated by commas.
- **Search domains**: added to names that have no dot, on IPv4 and IPv6 alike. A domain written as `~corp.example` is not added to names; it only sends that domain's lookups to this connection's servers. That is what a VPN to an office needs.
- **Servers in use**: what the connection is asking now.

Changes are not sent as you type. **Apply** checks them first, and says what
is wrong instead of saving anything half-filled. For example, Manually with no
address, or a router that is not an address. It then applies them to the
running connection without dropping it where NetworkManager can.
**Revert** throws the changes away.

### VPN

Each VPN and WireGuard connection, with a switch to connect and disconnect it,
and **Forget**. **Import…** adds one from a WireGuard `.conf` or an OpenVPN
`.ovpn` file. OpenVPN needs NetworkManager's OpenVPN plug-in
(`networkmanager-openvpn` or `NetworkManager-openvpn`, by distribution).

## Without NetworkManager

Without NetworkManager on the system bus the indicator is not drawn and the
pane says so. A tray applet for another network service, such as `iwgtk` for
iwd, still shows in the tray.
