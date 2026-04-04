#!/bin/bash
# Random notification generator for testing otto-islands
# Uses -a to set app_name so notifications group by app

notifications=(
  "thunar|File Manager|Your downloads folder has 847 files. At this point it's an archaeological dig."
  "thunar|File Manager|Move complete. 3 files had the same name. Good luck."
  "firefox|Firefox|Tab 47 is using 2GB of RAM. It's been open since March."
  "firefox|Firefox|Password breach detected. It was 'password123'. Again."
  "firefox|Firefox|A website wants to send you notifications. The audacity."
  "thunderbird|Thunderbird|You have 3 unread emails and 12,000 you're pretending don't exist."
  "thunderbird|Thunderbird|Reply All was clicked. Prayers up."
  "spotify|Spotify|Your Discover Weekly is ready. It's still mostly lo-fi hip hop."
  "spotify|Spotify|Your friend started a listening session. It's country."
  "vlc|VLC|Playback finished. Time to stare at the traffic cone in silence."
  "steam|Steam|Your friend is playing Elden Ring. They started 6 hours ago. They haven't moved."
  "steam|Steam|Sale: 90% off a game you already own. You're tempted anyway."
  "telegram|Telegram|Mom sent a photo. It's the dog again. 10/10 would notify."
  "telegram|Telegram|5 unread messages in the family group chat. All forwarded memes."
  "discord|Discord|Someone @everyone'd in #general. Chaos ensues."
  "discord|Discord|You were pinged 14 times while you were away. None were urgent."
  "gimp|GIMP|Export complete. The file is 4 pixels wider than you wanted."
  "nautilus|Files|Trash has 23GB of files you'll never restore but refuse to empty."
  "gnome-terminal|Terminal|Build succeeded on the 14th attempt. We don't talk about the other 13."
  "gnome-terminal|Terminal|Segfault. But hey, it compiled."
  "chromium|Chromium|A tab crashed. Honestly, it lived a good life."
  "libreoffice|LibreOffice|Document recovered. The formatting did not survive."
  "obs-studio|OBS Studio|Recording stopped. You forgot to unmute. Classic."
  "code|VS Code|Extension wants to reload. Again. For the 5th time today."
  "evolution|Calendar|Meeting in 5 minutes. You are not prepared."
  "blender|Blender|Render complete: 4 hours for a donut. Worth it."
)

echo "Sending random notifications every 5-15 seconds. Ctrl+C to stop."

while true; do
  idx=$((RANDOM % ${#notifications[@]}))
  IFS='|' read -r app_id title body <<< "${notifications[$idx]}"
  notify-send -a "$app_id" -i "$app_id" "$title" "$body"
  echo "[$app_id] $title: $body"
  sleep $((5 + RANDOM % 11))
done
