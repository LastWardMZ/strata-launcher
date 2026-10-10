# Servicio de cuentas de Strata

Guarda **nombres de usuario únicos**, el **buscador**, las **amistades**, los **bloqueos** y los **ajustes de
privacidad**. El chat y las partidas NO pasan por aquí: van por P2P entre launchers.

Corre en **Cloudflare Workers + D1** (plan gratuito, sin servidor propio ni mantenimiento).

## Cómo funciona la cuenta: sin contraseña

La cuenta ES la clave pública Ed25519 del launcher (la misma identidad del P2P). Cada petición lleva una firma:

```
Authorization: Strata <clave pública hex>.<ts en ms>.<firma hex>
firma = Ed25519( "strata-req\n<MÉTODO>\n<ruta con consulta>\n<ts>\n<sha256 hex del cuerpo>" )
```

No hay sesiones, tokens ni contraseñas que robar o filtrar. La hora debe estar a ±5 minutos de la del servidor
(protege de repeticiones antiguas). **Si se pierde la clave no hay recuperación** (no se guarda ningún correo):
la app ofrece guardar y restaurar una copia de seguridad.

## Desplegarlo

Necesitas Node.js y una cuenta de Cloudflare (gratuita, sin tarjeta). Desde la raíz del proyecto:

```powershell
.\scripts\deploy-accounts.ps1
```

Instala wrangler en una carpeta aparte, te pide iniciar sesión en Cloudflare, crea la base de datos D1, aplica
`migrations/` y publica. Al final muestra la URL (`https://strata-accounts.<tu-subdominio>.workers.dev`). Para que
una versión de la app la use por defecto:

```powershell
.\scripts\release.ps1 -Version <versión> -AccountsUrl https://strata-accounts.<tu-subdominio>.workers.dev -Publish
```

También se puede cambiar por usuario en *Ajustes → Perfil → Servidor de cuentas*.

### Límites del plan gratuito

100.000 peticiones al día. La app consulta cada 3 minutos mientras está abierta (unas 480 al día por usuario
activo), así que alcanza para cientos de usuarios. Si algún día crece, el plan de pago ($5/mes) lo multiplica.

## Probarlo en local

```powershell
cd code-main/apps/strata-accounts
node --test test/core.test.mjs                        # lógica, con una base de datos en memoria
npx wrangler d1 migrations apply strata-accounts --local
npx wrangler dev --local                              # el Worker real (workerd) en http://127.0.0.1:8787
```

La app apunta por defecto a `http://127.0.0.1:8787` si no se compila con `-AccountsUrl`.

## Qué guarda (datos personales)

Nombre de usuario, clave pública, biografía, ajustes de privacidad, fecha de creación, relaciones de amistad y
bloqueos. **No** guarda correos, contraseñas, mensajes ni contenido de partidas, y las direcciones IP solo se usan
al momento para los límites de uso (contadores con caducidad). Cada usuario puede borrar su cuenta desde la app
(`POST /me/delete`): se elimina todo lo anterior y el nombre queda libre.

## Reglas

- Nombres: de 3 a 24 caracteres (`A-Z a-z 0-9 _`), únicos sin distinguir mayúsculas, con una lista de nombres
  reservados (`admin`, `strata`, `modrinth`…). Un nombre puede parecerse visualmente a otro (`Erik` / `Erlk`):
  conviene moderar. Cambio de nombre: una vez al día.
- Una cuenta por launcher (por clave). Límites: 10 cuentas por hora y conexión, 30 búsquedas por minuto, 20
  solicitudes de amistad por hora.
- Privacidad: solicitudes de amistad (`everyone` / `mutual` = amigos de amigos / `none`), aparecer en el buscador, e
  invitaciones a partidas (`friends` / `none`, aplicada por el launcher que las recibe).

## API

Todas las rutas (salvo `GET /health`) requieren la firma de arriba. Respuestas en JSON; los errores llevan
`{error, code}` (`no-account`, `name-taken`, `has-account`, `clock`, `rate-limit`, `unauthorized`).

| Ruta | Descripción |
| --- | --- |
| `GET /health` | Estado del servicio |
| `POST /register` | `{username}` → crea la cuenta de esta clave |
| `GET /me`, `PATCH /me` | Perfil, biografía y `settings` |
| `GET /sync` | `{profile, social}` en una sola petición (lo que usa la app) |
| `POST /me/rename`, `POST /me/delete` | Cambiar el nombre y borrar la cuenta |
| `GET /users/search?q=` | Búsqueda por prefijo (mínimo 2 caracteres), con la relación de cada resultado |
| `POST /friends/request`, `/friends/accept`, `/friends/remove`, `/block`, `/unblock` | `{username}` |

Los amigos devueltos por `/sync` incluyen su `id`, que es su clave pública y por tanto su código de partidas.
