# Menma pack

NeoForge 1.21.1 modpack for the Menma server. Mods are downloaded from Modrinth/CurseForge; this repo only holds the pack list and configs.

## Friends: play in 3 steps
1. Install [Prism Launcher](https://prismlauncher.org) and sign in with your Microsoft account (you need to own Minecraft: Java Edition).
2. Download `Menma-Prism.zip` and drag it into Prism (or Add Instance, then Import).
3. Press Play. Every launch checks for updates and installs them automatically.

## Me: change the pack
```
cd pack
..\tools\packwiz\packwiz.exe mr add <modrinth-slug>
..\tools\packwiz\packwiz.exe remove <name>
cd ..
powershell -ExecutionPolicy Bypass -File scripts\publish.ps1 "what changed"
```
The server (`server\start.bat`) syncs its mods from the same pack every time it starts.
