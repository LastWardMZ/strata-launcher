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

## Licencia

Este proyecto es software libre bajo **GPL-3.0-only**, igual que la aplicación de Modrinth de la que parte
(ver `code-main/apps/app/LICENSE` y `code-main/COPYING.md`). Quien reciba el programa tiene derecho al código
fuente completo, que es este repositorio.

Strata es un proyecto independiente y **no está afiliado ni respaldado por Modrinth ni por Mojang/Microsoft**.
Minecraft es una marca de Mojang AB. Quedan en el árbol de código imágenes históricas del proyecto original
que están pendientes de retirar.
