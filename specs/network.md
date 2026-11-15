# Network

**Status:** draft  
**Related specs:** [topbar.md](topbar.md), [settings-app.md](settings-app.md), [context-menus.md](context-menus.md)

## Summary

Otto shows and changes the machine's network connection itself: an indicator in the top bar with a menu to pick a Wi-Fi network, and a Network pane in Settings for the details and the saved networks. NetworkManager is the backend. Otto keeps no network state of its own.

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
3. **The menu is for joining, Settings is for managing.** The menu has the switch, the networks, VPN toggles and the way to Settings. Forgetting a network, auto-join and the address details live in Settings.
4. **The list stays honest.** One row per network name, strongest access point first. Saved networks come before others. Hidden networks have no row. The list updates in place while the menu is open.

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
  - the wired connection's state, when there is a wired device
  - the networks in range, with the connected one ticked
  - VPN connections as toggles
  - disconnect
  - the Network pane in Settings
- Choosing a network connects to it. A saved network uses its saved profile. A new open network is joined directly. A new secured network asks NetworkManager to connect, and NetworkManager asks whichever secret agent is registered for the password.
- A Network pane in Settings with:
  - the Wi-Fi switch
  - the state of every device
  - the address details of the connection in use
  - the saved Wi-Fi networks, each with an auto-join checkbox and a way to forget it
  - whether the indicator is shown in the top bar
- Everything follows NetworkManager live. A change made with `nmcli`, another applet or the hardware switch shows up without reopening anything.

## Non-Goals

- **A secret agent of Otto's own (for now).** Until one lands (see Open Questions), the password for a new secured network comes from any registered agent, such as nm-applet's or `nmcli --ask`'s.
- **Creating or editing connection profiles:** static IP, proxies, 802.1X/enterprise settings, VPN import. `nm-connection-editor` and `nmtui` already do this. Settings can grow into it later.
- **Backends other than NetworkManager** (iwd alone, ConnMan, systemd-networkd). Without NetworkManager on the system bus the indicator does not appear, and a tray applet for that backend still works.
- **Bluetooth and mobile broadband.**

## Behavior

### Indicator

1. The indicator sits in the bar's status area, between the battery and the keyboard layout indicator.
2. It is shown when the `topbar.show_network` setting is on, NetworkManager is on the system bus, and the machine has a Wi-Fi or Ethernet device. Otherwise it takes no space.
3. What it draws, in order of precedence:
   - Wi-Fi off and no wired connection: the empty fan, crossed out.
   - A connection activating: the fan's bars light up one after another until the connection is up or fails.
   - Wired as the primary connection: the wired glyph.
   - Wi-Fi as the primary connection: the fan, with bars lit by signal strength:
     - below 25%: none
     - 25% to 49%: one
     - 50% to 74%: two
     - 75% and up: three
   - Nothing connected: the empty fan.
4. When NetworkManager reports limited connectivity or a captive portal, the glyph is drawn dimmed with a small exclamation mark.
5. A screen reader reads the indicator as a button with a menu. Its label says the state in words, for example "Wi-Fi, connected to Home, strong signal". It is announced when it changes.

### Menu

6. A click on the indicator opens the menu. Another click closes it. Opening it asks NetworkManager to scan, unless a scan happened in the last few seconds.
7. The menu reads, top to bottom:
   - **Wi-Fi**, ticked when the radio is on. Choosing it toggles the radio. When the hardware switch blocks the radio, the item is dimmed.
   - **Ethernet** (one line per wired device), saying whether it is connected or unplugged. Not selectable.
   - With Wi-Fi on, the networks in range:
     - Saved networks come first, then the others, which go into an **Other Networks** submenu when there are saved networks above them.
     - Each row shows the network's name and a signal icon. A secured network says *Secured* beside its name.
     - The connected network is ticked.
     - A network being connected to says *Connecting…* beside it.
   - **VPN** connections, each ticked while active. Choosing one starts or stops it.
   - **Disconnect from *name*** while Wi-Fi is connected.
   - **Network Settings…**, which opens Settings on the Network pane.
8. Choosing a network:
   - The connected one: nothing happens.
   - A saved one: that profile is activated on the Wi-Fi device.
   - Any other: a new profile is added and activated for that access point. Its security is filled in from what the access point advertises, and NetworkManager asks the registered secret agent for the password.
   - A WPA-Enterprise network cannot be joined from the menu. It is listed dimmed.
9. While the menu is open it follows NetworkManager. Networks appear and disappear, the tick moves, and the menu is redrawn in place. It is shown again at its new size only when the number of lines changes.
10. A failed connection is logged. The indicator goes back to the state NetworkManager reports.

### Settings pane

11. The **Network** pane sits after Displays in the sidebar.
12. **Wi-Fi** group:
    - the Wi-Fi switch
    - the network in use
    - *Show network status in the top bar*, bound to `topbar.show_network`
13. **Status** group: one line per Wi-Fi and Ethernet device, with its interface name and state (*Connected*, *Connecting*, *Disconnected*, *Cable unplugged*, *Unavailable*).
14. **Details** group, for the connection in use: name, kind, IPv4 address and prefix, router, DNS servers, IPv6 address, hardware address.
15. **Known networks** group:
    - every saved Wi-Fi network, the one in use first, then by last use
    - a switch per line for auto-join
    - a **Forget** button per line, which deletes the saved profile
16. Without NetworkManager the pane says so in a single line and shows nothing else.
17. The pane reads NetworkManager only while it is on screen, and follows its changes live while it is.

## Constraints & Edge Cases

- **Several access points share a network name.** They are one row, with the strongest one's signal and the strongest one used when a new profile is added.
- **Names that are not UTF-8.** They are shown with replacement characters and still matched by their bytes.
- **Two Wi-Fi devices.** The first managed one is used, and the Status group lists both.
- **NetworkManager restarts.** The indicator disappears and comes back once it has rejoined the bus.
- **Scans.** NetworkManager rate-limits them. A refused scan request is not an error.
- **Signal noise.** Strength changes are frequent. A burst of property changes is answered with a single re-read about 150 ms later.

## Rationale

- **The indicator is in otto-bar, not in a separate app or a tray applet.** The bar already draws its status indicators itself (battery, keyboard layout), because the tray carries only an icon, a tooltip and a flat menu. Drawing the indicator in the bar keeps it consistent with its neighbours, and the keyboard focus, menus and accessibility work as they do for every other indicator.
- **The model is shared.** One library (`otto-network`) reads NetworkManager into a snapshot and performs the actions. The bar and Settings both use it, so the two can never disagree about what "connected" means or how networks are sorted.
- **No password UI for now.** A correct secret agent is its own piece of work, with a dialog, credential storage hints and cancellation. NetworkManager already routes requests to any agent that is registered. Leaving it out keeps this change shippable without blocking anyone who has nm-applet's or another agent.

## Open Questions

- **Secret agent.** Where should Otto's live: in `otto-authorize`, next to the polkit agent and with the same password panel, or in otto-bar? It should support "show password" and hand passwords to NetworkManager's agent-owned storage.
- **Hidden networks.** Should *Join Other Network…* be a menu item that opens a sheet in Settings, or a dialog of its own?
- **Hotspot and airplane mode.**
