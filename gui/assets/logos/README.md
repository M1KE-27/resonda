# Logos de los servicios

Iconos de la pantalla de selección de servicios. Se incrustan en el binario (`gui/CMakeLists.txt`),
así que la app sigue siendo un único archivo.

- **SVG**: procedentes de [Simple Icons](https://simpleicons.org) (monocromos, `viewBox` 24×24).
  Cada uno lleva su color de marca como atributo `fill` en la etiqueta raíz. Los valores salen de
  los datos de Simple Icons, que a su vez los toman de las guías de marca oficiales de cada servicio.
  Las marcas cuyo color oficial es (casi) negro llevan `fill` blanco (variante para fondos oscuros).
- **`amazon-music.png`**: icono a todo color de 28×28 (Simple Icons ya no incluye Amazon Music).
  Un PNG de mayor resolución se vería más nítido.

Para añadir un servicio: deja aquí su `.svg` o `.png` y refiérelo en `gui/qml/ServiceSetup.qml`.

Los logotipos son marcas registradas de sus respectivos propietarios; Resonda es independiente y
no está afiliada a ninguno de ellos.

| Servicio | Color oficial | Color aplicado |
|---|---|---|
| YouTube | `#FF0000` | `#FF0000` |
| Spotify | `#1ED760` | `#1ED760` |
| Apple Music | `#FA243C` | `#FA243C` |
| SoundCloud | `#FF5500` | `#FF5500` |
| Bandcamp | `#408294` | `#408294` |
| Deezer | `#A238FF` | `#A238FF` |
| TIDAL | `#000000` | `#FFFFFF` |
| Vimeo | `#1AB7EA` | `#1AB7EA` |
| Twitch | `#9146FF` | `#9146FF` |
| TikTok | `#000000` | `#FFFFFF` |
| Instagram | `#FF0069` | `#FF0069` |
| Dailymotion | `#0A0A0A` | `#FFFFFF` |
| Facebook | `#0866FF` | `#0866FF` |
| Reddit | `#FF4500` | `#FF4500` |
| X | `#000000` | `#FFFFFF` |
