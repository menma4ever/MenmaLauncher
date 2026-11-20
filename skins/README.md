# Skins on the Menma server

Everyone shows their normal Microsoft account skin automatically.
To pick a different skin, join the server and type one of these in chat:

| What you want | Command |
|---|---|
| Any skin from a picture link | `/skin set web classic "https://link-to-skin.png"` (use `slim` instead of `classic` for thin arms) |
| Copy another Minecraft player's skin | `/skin set mojang <TheirName>` |
| Go back to your own skin | `/skin clear` |

Everyone sees the new skin right away. The skin must be a standard 64x64 Minecraft skin PNG.

**No link for your PNG?** Send it to the server owner: they save it here as `skins/<YourName>.png`,
run `scripts\publish.ps1 "add skin"`, and you use:
`/skin set web classic "https://raw.githubusercontent.com/menma4ever/MenmaLauncher/main/skins/<YourName>.png"`

(Server owner: you can set someone else's skin with the same commands plus their name at the end.)
