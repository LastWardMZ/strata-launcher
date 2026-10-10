# Servicio de cuentas de Strata

Guarda **nombres de usuario únicos**, el **buscador**, las **amistades**, los **bloqueos** y los **ajustes de
privacidad**. El chat y las partidas NO pasan por aquí: van por P2P entre launchers. Sin dependencias
externas; requiere Node.js 24 o posterior (`node:sqlite`).

## Ejecutarlo

```powershell
# En este equipo, para pruebas (escucha solo en 127.0.0.1)
$env:STRATA_ACCOUNTS_DB='C:\ruta\strata-accounts.sqlite'
node apps/strata-accounts/server.mjs
```

Variables de entorno:

| Variable | Por defecto | Qué hace |
| --- | --- | --- |
| `PORT` | `32145` | Puerto |
| `HOST` | `127.0.0.1` | Interfaz. Usa `0.0.0.0` para aceptar conexiones externas |
| `STRATA_ACCOUNTS_DB` | `./strata-accounts.sqlite` | Archivo de la base de datos |
| `STRATA_TRUST_PROXY` | `0` | Pon `1` detrás de un proxy: los límites de uso usan `X-Forwarded-For` |

Comprobación: `node --test apps/strata-accounts/server.test.mjs`

## Alojarlo para que lo usen otras personas

1. Despliégalo en cualquier sitio que ejecute Node 24 o Docker (`Dockerfile` incluido; el volumen `/data` guarda la
   base de datos).
2. Ponle **HTTPS** delante (Caddy, nginx, Cloudflare…). La app solo acepta `https://` salvo para este mismo equipo.
3. Activa `STRATA_TRUST_PROXY=1` si va detrás de un proxy.
4. Haz **copias de seguridad** del archivo SQLite.
5. Apunta la app a él: al compilar, `STRATA_ACCOUNTS_URL=https://tu-servidor` (con `scripts/release.ps1 -AccountsUrl …`),
   o por usuario en *Ajustes → Perfil → Servidor de cuentas*.

## Qué guarda (datos personales)

Nombre de usuario, hash de la contraseña (scrypt con sal propia), biografía, ajustes de privacidad, fecha de
creación, la clave pública de partidas del usuario (enlazada con una firma que demuestra que la posee), las
relaciones de amistad y los bloqueos. **No** guarda correos, direcciones IP (los límites de uso viven en
memoria), mensajes ni contenido de partidas. Cada usuario puede borrar su cuenta desde la app (`POST /me/delete`),
lo que elimina todo lo anterior y libera el nombre.

## Reglas

- Nombres: de 3 a 24 caracteres (`A-Z a-z 0-9 _`), únicos sin distinguir mayúsculas, con una lista de nombres
  reservados (`admin`, `strata`, `modrinth`…). Un nombre puede parecerse visualmente a otro (`Erik` / `Erlk`):
  conviene moderar los nombres.
- Contraseñas de 12 a 128 caracteres. **No hay recuperación de contraseña** (no se guarda ningún correo).
- Límites por IP y por usuario en autenticación, creación de cuentas, búsquedas y solicitudes de amistad.
- Privacidad: solicitudes de amistad (`everyone` / `mutual` = amigos de amigos / `none`), aparecer en el buscador, e
  invitaciones a partidas (`friends` / `none`, aplicada por el launcher que las recibe).

## API

Todas las respuestas son JSON. Salvo `register`, `login` y `health`, requieren `Authorization: Bearer <token>`.

| Ruta | Descripción |
| --- | --- |
| `GET /health` | Estado del servicio |
| `POST /register`, `POST /login` | `{username, password}` → `{token, user}` |
| `POST /logout` | Cierra la sesión |
| `GET /me`, `PATCH /me` | Perfil, biografía y `settings` |
| `POST /me/p2p` | Enlaza el código de partidas: `{code, ts, sig}`, firma Ed25519 de `strata-link:<id>:<code>:<ts>` |
| `POST /me/password`, `POST /me/delete` | Cambio de contraseña y borrado de la cuenta |
| `GET /users/search?q=` | Búsqueda por prefijo (mínimo 2 caracteres), con la relación de cada resultado |
| `GET /social` | Amigos (con su código de partidas), solicitudes y bloqueados |
| `POST /friends/request`, `/friends/accept`, `/friends/remove`, `/block`, `/unblock` | `{username}` |

Las sesiones duran 30 días y solo se guarda el hash SHA-256 del token.
