# Network

**Status:** draft  
**Related specs:** [topbar.md](topbar.md), [settings-app.md](settings-app.md), [context-menus.md](context-menus.md)

## Summary

Otto shows and changes the machine's network connection itself. The top bar has an indicator with a menu to pick a Wi-Fi network, and a dialog that asks for its password. Settings has a Network pane with Wi-Fi, Ethernet and VPN tabs, the link's details, and an editor for TCP/IP and DNS. NetworkManager is the backend. Otto keeps no network state of its own.

## What a good network manager does

Three are worth learning from:

- **macOS** (the Wi-Fi menu, and the Network pane in System Settings).
  - The menu bar icon is a fan with three bars, so you can read the signal without opening anything.
  - The menu opens with a Wi-Fi switch, then the connected network, then *Known Networks*, then *Other Networks* folded behind a disclosure. *Other…* joins a hidden network, and *Wi-Fi Settings…* is at the bottom.
  - Each row shows its own signal and a lock when the network is secured.
  - A network appears once, however many access points broadcast it.
  - The menu starts a scan when it opens, and keeps updating while it is open.
  - Settings lists every service (Wi-Fi, Ethernet, VPN) with its status. Under each it shows the details (IP address, router, DNS, hardware address) and the known networks, which you can forget or set to auto-join.
- **GNOME** (Quick Settings and the Wi-Fi and Network panels).
  - The same model with a switch per technology, and airplane mode.
  - The shell answers NetworkManager's password requests itself, as a secret agent, so a password dialog never comes from a separate applet.
- **KDE plasma-nm.**
  - The richest list (expandable rows with details and a speed graph), and VPNs in the same list.
  - It also shows how much screen it costs: a list that wide is hard to scan in a menu.
- **nm-applet** is what Otto shows today through the tray. Its flat dbusmenu has no signal bars, no layout and a GTK password dialog. It is the baseline to beat.

The common ground, which Otto adopts:

1. **NetworkManager owns everything.** Connections, auto-connect, credentials and scanning all stay with NetworkManager. The UI reads its state and asks it to change. It never keeps a copy of its own.
2. **One glance tells the state.** The bar shows Wi-Fi signal in bars, wired as its own glyph, and a distinct look when there is no network, when the radio is off, and when the connection is up but the internet is not reachable.
3. **The menu is for joining, Settings is for managing.** The menu has the switch, the networks, VPN toggles and the way to Settings. Forgetting a network, auto-join, addresses and DNS live in Settings.
4. **The list stays honest.** One row per network name, strongest access point first. Saved networks come before others. Hidden networks have no row. The list updates in place while the menu is open.
5. **What macOS hides, Otto shows.** macOS keeps band, channel, link rate and BSSID behind Option-click, and its TCP/IP sheet edits one family at a time behind a Details… button. Otto puts the link details on the Wi-Fi tab and a line under the connected network in the menu. IPv4, IPv6 and DNS sit on the tab of the connection they belong to, with edits checked before they are applied.

## Goals

- An indicator in the top bar's status area that shows:
  - Wi-Fi signal (no bars, or one to three bars)
  - a wired connection
  - connecting
  - no connection
  - the radio off
  - connected with no internet
- Clicking the indicator opens a menu with:
  - the Wi-Fi switch
  - the networks in range, with the connected one ticked
  - the wired connection's state, when there is a wired device
  - VPN connections as toggles
  - disconnect
  - the Network pane in Settings
- Choosing a network connects to it. A saved network uses its saved profile. A new open network is joined directly. A new secured network first asks for its password in Otto's own dialog.
- *Join Other Network…* joins a network that does not broadcast its name.
- Otto is a NetworkManager secret agent. When NetworkManager needs a Wi-Fi password it does not have, Otto asks for it in the same dialog, so Wi-Fi needs no other applet. A Wi-Fi key that a previous agent kept (agent-owned) is handed to NetworkManager to keep from then on.
- A Network pane in Settings, with a tab each for Wi-Fi, Ethernet and VPN:
  - the Wi-Fi switch
  - the link: signal, band and channel, link rate, security, access point, hardware address
  - each wired device's state, speed and hardware address
  - TCP/IP for the connection: IPv4 and IPv6 each Automatic, Manual, Link-local, Shared or Off, with addresses and router when manual
  - DNS: whether to also use the network's servers, the servers to ask first, search domains, and the servers in use
  - the saved Wi-Fi networks, each with an auto-join switch and Forget
  - for the Wi-Fi or Ethernet profile shown, its private hardware address and whether it is metered
  - whether the connection in use counts as metered now, set or guessed
  - a saved Wi-Fi network's password, shown with a QR code a phone scans to join
  - the VPNs, each with a switch to connect and Forget, and Import… for a WireGuard or OpenVPN file
- Whether the indicator is shown is a switch on the Top bar pane, next to the clock's, so it is there whatever devices the machine has.
- Everything follows NetworkManager live. A change made with `nmcli`, another applet or the hardware switch shows up without reopening anything.

## Non-Goals

- **802.1X (WPA Enterprise) and VPN plug-in secrets.** The agent answers Wi-Fi personal and WEP keys and declines the rest, so another agent can answer. Such profiles connect without an agent when their secrets are stored for all users.
- **Proxies, MTU, an explicit cloned MAC address, routes, bonds, bridges.** `nm-connection-editor` and `nmtui` do these.
- **Backends other than NetworkManager** (iwd alone, ConnMan, systemd-networkd). Without NetworkManager on the system bus the indicator does not appear, and a tray applet for that backend still works.
- **Bluetooth and mobile broadband.**

## Behavior

### Indicator

1. The indicator sits in the bar's status area, between the battery and the keyboard layout indicator.
2. It is shown when the `topbar.show_network` setting is on, NetworkManager is on the system bus, and the machine has a Wi-Fi or Ethernet device. Otherwise it takes no space.
3. What it draws, in order of precedence:
   - Wi-Fi off and no wired connection: the empty fan, crossed out. Wi-Fi counts as off when NetworkManager's switch is off, and also when a kill switch holds the radio off, whatever NetworkManager's switch says.
   - Wired as the primary connection: the wired glyph.
   - Any other primary connection, over a link that is up: the fan for a Wi-Fi link, with bars lit by signal strength, or the wired glyph for a connected cable:
     - below 25%: none
     - 25% to 49%: one
     - 50% to 74%: two
     - 75% and up: three
   - A connection activating: the fan's bars light up one after another until the connection is up or fails. A link in use keeps its glyph while another one activates.
   - Nothing connected: the empty fan.
4. When NetworkManager reports limited connectivity or a captive portal, the glyph is drawn dimmed with a small exclamation mark.
5. A screen reader reads the indicator as a button with a menu. Its label says the state in words, for example "Wi-Fi, connected to Home, strong signal". It is announced when it changes.

### Menu

6. A click on the indicator opens the menu. Another click closes it. Opening it asks NetworkManager to scan, unless a scan happened in the last few seconds.
7. The menu reads, top to bottom:
   - **Wi-Fi**, ticked when the radio is actually on. Choosing it toggles the radio.
     - When a soft block holds the radio off (the airplane key, `rfkill block wifi`, another desktop's switch), the item is unticked, with *Off with airplane mode* under it. Choosing it lifts the block and turns Wi-Fi on.
     - When a hard block holds it off (a physical switch, the firmware), the item reads *Wi-Fi — off with the wireless switch* and is dimmed: only that switch can turn it on.
     - The list of networks shows only while the radio is on.
   - With Wi-Fi on, the networks in range:
     - Saved networks come first. Each row shows the network's name and a signal icon. A secured network says *Secured* beside its name.
     - The connected network is ticked, with a line under it giving its band, channel, link rate and IPv4 address.
     - A network being connected to says *Connecting…* beside it.
   - **Other Networks**, a submenu holding the networks that are not saved, when there are saved networks above it. Without saved networks they are listed directly.
   - **Join Other Network…**, with Wi-Fi on.
   - **Ethernet** (one line per wired device), saying whether it is connected or unplugged. Not selectable.
   - **VPN** connections, each ticked while active. Choosing one starts or stops it.
   - **Disconnect from *name*** while Wi-Fi is connected.
   - **Network Settings…**, which opens Settings on the Network pane.
8. Choosing a network:
   - The connected one: nothing happens.
   - A saved one: that profile is activated on the Wi-Fi device.
   - An open one: a new profile is added and activated for that access point.
   - A secured one: the password dialog opens first. Join adds and activates a profile carrying the key: WPA2 (`wpa-psk`), or SAE when the access point offers only WPA3, or WEP. NetworkManager stores the key with the profile.
   - A WPA-Enterprise network cannot be joined from the menu. It is listed dimmed.
9. While the menu is open it follows NetworkManager. Networks appear and disappear, the tick moves, and the menu is redrawn in place. It is shown again at its new size only when the number of lines changes.
10. A failed connection is logged. The indicator goes back to the state NetworkManager reports.

### Password dialog

10a. A card on the overlay layer, centred, holding the keyboard while it is up. It shows a Wi-Fi badge (with a padlock for a secured network), a title, one line saying what is asked, then:
   - for a hidden network: *Network name* and a segmented *None / WPA2 / WPA3 / WEP*
   - *Password* (masked), with *Show password*, unless the network is open
   - *Cancel* and *Join*. Join is dimmed until the password can be a key of that kind: 8–63 characters or 64 hex digits for WPA2, any for WPA3, 5 or 13 characters or 10 or 26 hex digits for WEP.
10b. Return joins, Escape cancels, Tab moves between the name and the password.
10c. Otto registers as a NetworkManager secret agent (`org.otto.bar`) on the system bus, again whenever NetworkManager restarts. For a Wi-Fi personal or WEP key, when NetworkManager allows interaction, it opens the dialog. When NetworkManager says the key it had was refused, the dialog says so. The typed key goes back in the reply, and only `psk`, `wep-key0` and `wep-key-type` are ever sent; nothing is logged. Otto keeps no store of its own: when the profile's `psk-flags` or `wep-key-flags` has `AGENT_OWNED` (0x1) and not `NOT_SAVED` (0x2), Otto then updates the profile (`Settings.Connection.Update`, read fresh with `GetSettings`) with that flag cleared and the key included, so NetworkManager keeps it and later connects, auto-connect at login included, need no agent. NetworkManager never passes an agent's own answer back to `SaveSecrets`, so without this an agent-owned profile would ask on every connect and never auto-connect. When the update is refused (no permission to change the profile), nothing changes and the next connect asks again. `SaveSecrets` and `DeleteSecrets` are accepted and do nothing. Other secrets, and requests that allow no interaction, are declined at once. A request NetworkManager withdraws closes the dialog if it is on screen, and is answered no if it is still waiting; a withdrawal belongs to its own request and never closes a later one. Only one dialog is up at a time: a second request waits until the first is answered. A call from anyone but NetworkManager's own bus name is refused.

### Settings pane

11. The **Network** pane sits after Displays in the sidebar. A segmented control at the top holds a tab for Wi-Fi when there is a Wi-Fi device, for Ethernet when there is a wired one, and for VPN always. It opens on the kind of the connection in use.
12. **Wi-Fi tab:**
    - the Wi-Fi switch, on only while the radio is, and the network in use. Under a soft block the switch is off and turning it on lifts the block. Under a hard block it is dimmed. Each says which it is.
    - **Connection**, while connected:
      - signal in bars, in words and as a percentage
      - band and channel
      - link rate
      - security
      - access point (BSSID)
      - hardware address
      - *Metered*, when NetworkManager counts the link as metered: *Yes*, or *Yes (guessed)* when it guessed with nothing set on the profile (the device's `Metered` property: `yes`, `guess-yes`)
      - a note when the internet cannot be reached
    - **Options**, **TCP/IP** and **DNS** for a saved network: the one in use, or the one picked in *Settings for*, which heads Options
    - **Known networks:** every saved Wi-Fi network, the one in use first, then by last use. Each has an auto-join switch and *Forget*.
13. **Ethernet tab:** each wired device's state with its speed, its hardware address, and *Metered* as on the Wi-Fi tab while it is connected. Then Options, TCP/IP and DNS for the wired profile in use, or the most recently used one.
14. **VPN tab:** each VPN or WireGuard connection with a switch that connects and disconnects it, its state, and *Forget*. *Import…* opens the file chooser for a WireGuard `.conf` or OpenVPN `.ovpn` and imports it with `nmcli connection import`; NetworkManager has no D-Bus call for importing.
15. **TCP/IP group:**
    - *Configure IPv4*: Automatic (DHCP), Manually, Link-local only, Share this connection, Off
    - *Configure IPv6*: Automatic, Automatic DHCP only, Manually, Link-local only, Share this connection, Off
    - Manually shows the family's addresses (`address/prefix`, comma-separated; a missing prefix is /24 or /64) and router as fields. Any other method shows the addresses and router in use, for IPv6 too.
16. **DNS group:**
    - *Automatic DNS*: off means only the servers below are asked
    - *DNS servers*: IPv4 and IPv6 together, comma-separated
    - *Search domains*, saved on both families so they apply on an IPv6-only connection too: a `~` prefix makes a routing-only domain
    - the servers in use
16a. **Options group**, saved as soon as a choice is made, each on its own update of the profile:
    - *Private Wi-Fi address* (*Private address* for Ethernet), a pop-up of three choices, writing the link setting's cloned MAC address (`802-11-wireless` or `802-3-ethernet`):
      - Off: `permanent`, the device's own address
      - Fixed for this network: `stable`, generated once per profile
      - Changes over time: `random`, new on each connect
    - A profile with nothing set shows *Default*, one with `preserve` shows *As the device has it*, and one with an explicit address shows that address. None of them is changed until a choice is picked.
    - The new address is used the next time the connection comes up; the row says so. It is not reapplied, since NetworkManager cannot change the address of a link that is up without taking it down.
    - *Metered connection*, a pop-up writing `connection.metered`: Automatic (`0`, NetworkManager guesses), On (`1`), Off (`2`). It is applied to the running connection in place (`Reapply`). Where NetworkManager cannot, it waits for the next connect: being metered is no reason to drop a link.
    - *Password* (Wi-Fi only), with *Show…*:
      - It reads the profile with `GetSettings`. A password is shown only after the person confirms it's them: before `GetSecrets`, Settings asks polkit's `CheckAuthorization` for `org.otto.network.reveal-password` (`resources/polkit/org.otto.network.policy`), naming its own process as a `unix-process` subject (pid and start time) with `AllowUserInteraction`. The action is `auth_self_keep` for an active session and `auth_self` otherwise, so the session's agent (otto-authorize) asks for the person's own password, and a second reveal within polkit's few minutes asks nothing. Otto caches no answer itself.
      - Only an authorisation goes on to the secrets. Not authorised or dismissed, polkit not on the bus, or the action unknown because the policy is not installed: all refuse, and nothing is read.
      - Then it reads the secrets with `Settings.Connection.GetSecrets("802-11-wireless-security")`, sent allowing interactive authorisation. NetworkManager checks its own polkit action as well (`org.freedesktop.NetworkManager.settings.modify.own` for a profile visible only to this user, `settings.modify.system` otherwise), which an active session usually holds outright, and a polkit agent may prompt again. While either call is out the row says it is reading and the button is dimmed.
      - Neither check nor `GetSecrets` is made when the profile stores no password: open, enterprise, or a key whose flags say agent-owned (`0x1`) or not saved (`0x2`). NetworkManager would return nothing for those.
      - A refusal by either check (`PermissionDenied` from NetworkManager) says the password may not be read. A password not stored says so. An enterprise network says it signs in with an account. Each shows on the row's detail line, as other refusals do.
      - Otherwise a sheet comes up with the network's name, its password, and a QR code holding `WIFI:T:<kind>;S:<name>;P:<password>;;`, plus `H:true;` for a hidden network. The kind is `WPA` for WPA/WPA2 (`wpa-psk`), `SAE` for WPA3-only (`sae`), `WEP` for WEP (the key `wep-tx-keyidx` names), and `nopass` for open and Enhanced Open, which leave out `P:`. `\`, `;`, `,`, `:` and `"` in the name and password are escaped with a backslash. A string too long for a code (more than version 10 holds) shows the password with a line saying so.
      - The sheet has one button, *Done*. Done, Escape or Return close it, and closing it drops the password from the pane's state. The password is never logged, and the types holding it (`Password`, `WifiShare`, the QR symbol) print nothing of it in `Debug`.
    - A refusal shows on the row's detail line.
17. Edits to TCP/IP and DNS are a draft. *Changes* has *Revert* and *Apply*, dimmed while there is nothing to apply. Apply checks the draft first:
    - Manually needs at least one address.
    - Every address, router and DNS server must be of its family. A router is checked only with Manually, the one method that uses it.
    - Every search domain must be a domain.
    A problem is written under *Changes*, and nothing is sent. A draft that passes is saved on the profile and applied to the running connection in place (`Reapply`), or by reconnecting where NetworkManager cannot apply it in place.
18. Without NetworkManager the pane says so in a single line and shows nothing else.
19. The pane reads NetworkManager only while it is on screen, and follows its changes live while it is.

## Constraints & Edge Cases

- **Several access points share a network name.** They are one row, with the strongest one's signal. The row asks for the strongest security among them, and a new profile is added for the strongest access point that takes that key.
- **A hidden network in use.** Its access point may report no name. It is shown under its saved profile's name and reads as connected.
- **Names that are not UTF-8.** They are shown with replacement characters and still matched by their bytes.
- **Two Wi-Fi devices.** The first managed one with a connection is used for the menu and the Wi-Fi tab, or the first when none has one; the other is not shown. The Status group is on the Ethernet tab and lists wired devices only.
- **NetworkManager restarts.** The indicator disappears and comes back once it has rejoined the bus.
- **A failed read.** If NetworkManager fails a read, it is tried again on its next change or after a pause that grows from 1 s to 30 s. If following stops (the bus went away), the bar and Settings start following again.
- **Kill switches.** The radios' blocks are read from `/sys/class/rfkill`, and their changes followed on `/dev/rfkill`, since NetworkManager does not report every soft block set elsewhere. A soft block is lifted by writing to `/dev/rfkill`, which the active session may (systemd's `uaccess`). Where it may not, turning Wi-Fi on is left to NetworkManager.
- **Sleep.** While the machine sleeps NetworkManager takes the devices down and the indicator may go with them. When logind says the machine has woken (`PrepareForSleep(false)`), everything is read again, so the indicator and an open menu or pane catch up at once. Going to sleep reads nothing.
- **Scans.** NetworkManager rate-limits them. A refused scan request is not an error.
- **Signal noise.** Strength changes are frequent. A burst of changes is gathered for about 150 ms and answered once, and only as much as it needs is read again: a burst of signal strengths alone is applied from the signals themselves with no read, and saved connections are read again only when one of them changed.
- **Old and new spellings.** IPv4 addresses are read from `address-data`, or the older `addresses`, and written as `address-data`. DNS is read from `dns-data` (NetworkManager 1.42), or the older `dns`. It is written as `dns-data` where the connection already uses it or an entry needs it (a port, an IPv6 scope such as `%eth0`, or a DNS-over-TLS name such as `#one.one.one.one`), and as `dns` otherwise, which every NetworkManager reads. Each value is written in one spelling only, so neither can shadow the other, and every property not edited is kept.
- **Cloned MAC on D-Bus.** The setting libnm and `nmcli` call `cloned-mac-address` travels on D-Bus as `assigned-mac-address` (a string: `permanent`, `stable`, `random`, `preserve` or an address), while the D-Bus `cloned-mac-address` holds only an explicit address as bytes. The string is read first, the bytes as a fallback, and only the string is written, the bytes removed.
- **One update.** IPv4 and IPv6 are saved in a single update and applied once, so one family is never saved without the other.
- **Secrets on update.** A profile's settings are read without secrets, and an update without secrets keeps the ones NetworkManager stored.
- **QR codes.** `otto_kit::qr` encodes byte mode at error correction level M, versions 1 to 10 (up to 213 bytes), with the mask the standard's penalty rules pick; `otto_kit::components::qr_code` draws it black on white with a four-module quiet zone, on whole device pixels. The longest name (32 bytes) and password (63 characters) fit unless nearly all of their characters need escaping. No QR library is linked.

## Rationale

- **The indicator is in otto-bar, not in a separate app or a tray applet.** The bar already draws its status indicators itself (battery, keyboard layout), because the tray carries only an icon, a tooltip and a flat menu. Drawing the indicator in the bar keeps it consistent with its neighbours, and the keyboard focus, menus and accessibility work as they do for every other indicator.
- **The model is shared.** One library (`otto-network`) reads NetworkManager into a snapshot and performs the actions. The bar and Settings both use it, so the two can never disagree about what "connected" means or how networks are sorted.
- **The secret agent lives in otto-bar.** The bar runs for the whole session and already holds the network connection on the system bus, and the dialog is the one the menu opens. A second process would need its own bus connection and its own way to put a dialog on screen.
- **The key travels with the profile.** Joining a new network sends the key with the new profile, so NetworkManager stores it and needs no agent to reconnect later. The agent covers the cases that remain: a changed password, a profile saved without one, or one whose key a previous agent kept, which it moves to NetworkManager the first time it is answered.
- **TCP/IP edits are a draft.** Applying each field as it is typed would drop the connection the moment Manually is picked, before any address is in. A draft lets the person fill in a whole configuration, have it checked, and apply it once.

## Open Questions

- **Hotspot and airplane mode.**
- **Proxies.** Per-connection proxy settings (`proxy` setting) would fit the TCP/IP tab.
