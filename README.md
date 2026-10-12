# Strata

Launcher de Minecraft Java con servidores personales, amigos y partidas P2P, basado en la
[aplicación de Modrinth](https://github.com/modrinth/code) (Tauri 2 + Rust + Vue 3).

> **Estado:** versión de pruebas. Puede fallar y cambiar sin aviso.

## Qué añade respecto a Modrinth App

- **Servidores personales**: crea y gestiona servidores locales (vanilla, Paper, Folia, Velocity, Purpur,
  Fabric, Forge, NeoForge, BungeeCord), con consola en vivo, copias de seguridad y modpacks `.mrpack`.
- **Cuenta propia de Strata**, sin contraseña: tu cuenta es la clave de tu launcher. Nombres de usuario únicos,
  buscador de amigos, solicitudes con privacidad (todos / amigos de amigos / nadie) y bloqueos. El servicio
  (`code-main/apps/strata-accounts`) corre en Cloudflare Workers + D1, gratis y sin servidor propio; guarda solo
  nombres y amistades.
- **Chat y partidas P2P** directos entre launchers (el chat y las partidas no pasan por el servidor):
  invitaciones, túneles TCP y mundos LAN o servidores personales compartidos con amigos
  ([iroh](https://github.com/n0-computer/iroh); si no hay conexión directa, por relays públicos cifrados). Sin
  cuenta también se puede añadir amigos pegando su código.
- **Grabación de partidas** (solo Windows): una tecla configurable inicia y detiene la grabación de Minecraft
  aunque el juego esté en primer plano, con audio del juego (y micrófono opcional) y un indicador «REC» que no sale
  en el vídeo. Los vídeos aparecen en *Capturas → Vídeos*, con reproductor y un editor para recortarlos. Detalles en
  [Grabación](#grabación-de-partidas).
- **Color de acento** a elección del usuario en *Ajustes → Apariencia*.
- **Sin telemetría** de terceros: sin anuncios, analítica ni informes de errores a servicios externos.
- **Actualizaciones automáticas** firmadas desde las Releases de este repositorio.

## Instalar

Descarga `Strata-Setup-<versión>.exe` de la página de [Releases](../../releases/latest) y ejecútalo.
El instalador no está firmado con certificado de Windows, así que SmartScreen puede avisar:
*Más información → Ejecutar de todas formas*. Una vez instalado, Strata se actualiza solo.

## Compilar

Requisitos en Windows: Rust (MSVC), Visual Studio Build Tools, Node.js, pnpm y WebView2.

```powershell
cd code-main
pnpm install --frozen-lockfile
pnpm --filter "@modrinth/app" build
```

`packages/app-lib/.env.prod` aporta las variables de compilación del código base. Para publicar una versión
con actualizador (necesita la clave de firma privada del mantenedor) está `scripts/release.ps1`.

## Grabación de partidas

*Ajustes → Grabación*: se activa, se elige la tecla (por defecto **F9**; avisa si choca con un atajo de Minecraft),
los FPS (30/60), la calidad, la resolución, el formato (MP4/MKV), el audio, el micrófono, el codificador y la carpeta.
La tecla solo se registra mientras hay un Minecraft lanzado desde Strata.

Cómo funciona y por qué se eligió así:

- **Vídeo**: [FFmpeg](https://ffmpeg.org) con el filtro `gfxcapture` (Windows.Graphics.Capture) captura solo la ventana
  del juego por GPU, sin mod ni OBS. El mismo FFmpeg codifica (NVENC, AMF, Quick Sync, Media Foundation o x264; se
  prueban al activarlo y se usa el primero que funciona), recorta y genera miniaturas.
- **Audio**: FFmpeg no tiene captura de audio de Windows, así que se usa WASAPI (crate [`wasapi`](https://crates.io/crates/wasapi),
  MIT) con captura **por proceso**: solo suena Minecraft, no el resto del sistema (Windows 10 2004 o posterior).
- **Tecla global**: `RegisterHotKey`, que funciona con cualquier ventana en primer plano.
- **Componente**: FFmpeg 9.0.2 «essentials» de [gyan.dev](https://www.gyan.dev/ffmpeg/builds/) (licencia GPLv3, compatible
  con la de Strata) se descarga **una sola vez** al activar la grabación desde su
  [publicación en GitHub](https://github.com/GyanD/codexffmpeg/releases/tag/9.0.2), se verifica con una huella SHA-256
  fijada en el código (`apps/app/src/api/recording/ffmpeg.rs`) y se guarda en la carpeta de datos de la app. No se incluye
  en el instalador.
- **Archivos**: se graba fragmentado (sobrevive a un cierre inesperado del juego o del launcher) y al terminar se
  convierte en un MP4 normal sin recodificar. Las grabaciones interrumpidas se recuperan solas. Los vídeos se guardan
  por instancia en `<instancia>/recordings` (o en una subcarpeta por instancia de la carpeta elegida) y nunca se
  sobrescribe nada; los recortes son archivos nuevos y el original no se toca.

Limitaciones conocidas: solo Windows; la sincronización entre audio e imagen se midió en pruebas controladas
(±35 ms con una referencia limpia) pero no con Minecraft; los vídeos MKV se graban pero la reproducción dentro del
launcher depende del soporte de WebView2.

## Licencia

Este proyecto es software libre bajo **GPL-3.0-only**, igual que la aplicación de Modrinth de la que parte
(ver `code-main/apps/app/LICENSE` y `code-main/COPYING.md`). Quien reciba el programa tiene derecho al código
fuente completo, que es este repositorio.

Strata es un proyecto independiente y **no está afiliado ni respaldado por Modrinth ni por Mojang/Microsoft**.
Minecraft es una marca de Mojang AB. Quedan en el árbol de código imágenes históricas del proyecto original
que están pendientes de retirar.
