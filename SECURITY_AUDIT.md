# Auditoría de seguridad de Strata

Fecha: 2026-10-12 · Versión auditada: 0.4.1 (árbol de trabajo, sin publicar) · Alcance: código propio de Strata
(`apps/app`, `apps/app-frontend`, `apps/strata-accounts`) y los puntos del núcleo heredado de Modrinth
(`packages/app-lib`, `packages/utils`) por los que pasan datos externos.

**Estado de la Fase 2:** terminada. Cada arreglo es un commit propio (`[SEC-nn]` en el mensaje) sobre el commit base
`479840a` (el estado de la 0.4.1 antes de arreglar nada), con su test. Sin publicar ni hacer `push`.
La suite completa de Rust (75 pruebas) y la del Worker (10) pasan.

Leyenda de verificación: **Verificado** = lo comprobé ejecutando código o leyendo el código completo del camino;
**Por lectura** = deducido leyendo el código, sin ejecutarlo; **No verificado** = no pude confirmarlo.

## Resumen

| ID | Gravedad | Hallazgo | Estado |
|---|---|---|---|
| SEC-01 | Alta | Un mensaje de chat en el log de Minecraft puede redirigir el túnel P2P a otro puerto local | Arreglado · `fb4d11d` |
| SEC-02 | Media | La privacidad «amigos de amigos» y los bloqueos de la cuenta no se aplican en el P2P | Arreglado · `5f3f55b` |
| SEC-03 | Media | `strata-local://launch/instance/…?server=` lanza el juego y entra en un servidor sin confirmar | Arreglado · `dcca3ad` |
| SEC-04 | Media | Servidores y instaladores de Purpur, Fabric, BungeeCord, Forge y NeoForge se descargan sin comprobar hash | Arreglado en Forge/NeoForge · `5bec322` |
| SEC-05 | Media | Un amigo puede inflar disco y CPU con mensajes de chat (id sin límite, archivo reescrito entero) | Arreglado · `5a3a135` |
| SEC-06 | Media | Worker: sondeo de nombres de usuario sin límite de uso (revela cuentas «no buscables») | Arreglado · `304d66f` (hay que redesplegar el Worker) |
| SEC-07 | Baja | P2P: sin límite de conexiones/streams de pares desconocidos; nombres de solicitud suplantables | Arreglado · `77016a2` |
| SEC-08 | Baja | ZIP/modpack: el tamaño declarado no acota la descompresión real (zip bomb) | Arreglado · `b09c716` |
| SEC-09 | Baja | Worker: una petición firmada se puede repetir durante 5 minutos (sin nonce) | Descartado por ahora (tu decisión) |
| SEC-10 | Baja | Worker: sin límite para peticiones inválidas o autenticadas: se puede agotar la cuota gratuita | Pendiente de tu decisión (panel de Cloudflare) |
| SEC-11 | Baja | Tuberías del audio de la grabación con el control de acceso por defecto | Arreglado · `a5a39e5` |
| SEC-12 | Baja | Restos de configuración: HTTP en claro a `*.taila228c5.ts.net`/`localhost:8000`; CSP sin `base-uri`/`form-action` | Arreglado salvo `http://textures.minecraft.net` · `f4b37ed` |
| SEC-13 | Baja | Límite de túneles por amigo no atómico | Arreglado · `ecac2ef` |
| SEC-14 | Info | La clave de la cuenta (`identity.key`) se guarda sin cifrar y se puede exportar a cualquier ruta | Documentado, sin cambios |
| SEC-15 | Info | Dependencias: avisos que alcanzan el launcher (ninguno explotable de forma práctica) | Documentado, sin cambios |

## Qué se revisó y está bien

- **Inyección SQL en el Worker (D1):** todas las consultas usan `bind()`; la búsqueda valida `^[a-zA-Z0-9_]+$` y escapa
  `%`, `_` y `\` (`apps/strata-accounts/src/core.mjs:195-205`). Por lectura.
- **Autenticación del par P2P:** `remote_id` sale de la conexión QUIC autenticada de iroh (clave pública), no de un campo del
  mensaje (`p2p.rs:404`). Por lectura.
- **Instancias recibidas por el chat:** viajan solo como «receta» (versión, cargador, ids de modpack): no hay transferencia de
  archivos, así que no hay zip slip ni rutas por ahí. Los campos se limpian (`p2p.rs:203-232`) y el icono solo se acepta del
  CDN de Modrinth. Por lectura.
- **MOTD de servidores remotos (`v-html`):** probé el parser con cargas hostiles (`<img onerror>`, colores con comillas y
  `url(javascript:)`): escapa el HTML y descarta los colores no válidos. Verificado.
- **Markdown remoto:** pasa por `xss` con lista blanca (`packages/utils/parse.ts:232`); no hay otros `v-html` con datos externos
  en `apps/app-frontend` (los demás son textos fijos). Por lectura.
- **Enlaces:** el manejador de clics solo abre `http(s)`, `mailto` y `tel` (`App.vue:2105-2125`).
- **Importar ZIP y packs `.mrpack` en servidores locales:** rechazan `..`, rutas absolutas y prefijos de unidad
  (`local_servers.rs:244-262`, `local_server_packs.rs:33-39`), no sobrescriben (`create_new`), usan lista de dominios de
  descarga y exigen SHA-512/SHA-1 (`local_server_packs.rs:42-70`). Por lectura.
- **Archivos de servidores locales:** `resolve_file` canonicaliza y exige quedarse dentro de la carpeta, lo que también
  neutraliza enlaces simbólicos y junctions (`local_servers.rs:555-575`). Por lectura.
- **Actualizador:** firma minisign obligatoria (clave pública en `tauri.conf.json:69`), endpoint HTTPS, ningún
  `allowDowngrades` en el frontend, y la clave privada no está en el repositorio (`.gitignore` excluye `*.key`; ningún
  `.key` rastreado). Por lectura.
- **Secretos en el repo:** los `.env*` rastreados solo contienen URLs públicas de Modrinth. Verificado.
- **FFmpeg de la grabación:** URL fija y SHA-256 fijado (`recording/ffmpeg.rs:17-19`), argumentos sin shell.

## Hallazgos

### SEC-01 · Alta · Un mensaje de chat redirige el túnel P2P a otro puerto local
- **Dónde:** `apps/app/src/api/p2p.rs:1227-1242` (`LAN_PORT_MARKERS`, `lan_port_from_log`), usado en `:729` para decidir
  el puerto al que conecta cada túnel de un mundo LAN compartido.
- **Qué pasa:** el puerto se lee de `logs/latest.log` buscando frases como «Started serving on » **en cualquier parte de
  cualquier línea**, incluidas las de chat. Después solo se comprueba que ese puerto esté escuchando.
- **Verificado:** ejecuté una copia literal de la función: con el log
  `…<Steve> Started serving on 3306` devuelve `Some(3306)`, y con el log honesto `Some(61234)`.
- **Escenario realista:** tú abres un mundo a LAN y lo compartes con tus amigos. Cualquier jugador que esté en ese mundo
  escribe en el chat «Started serving on 3306» (o 2375, 5432, 8000…). Strata pasa a reenviar el túnel de **todos** tus amigos
  a ese puerto de tu `127.0.0.1` si hay algo escuchando. Resultado: un amigo (o alguien que entra a tu mundo) llega a
  servicios locales tuyos que no eran para él (bases de datos de desarrollo, APIs de Docker, consolas de servidores).
  También puede pasar con cualquier línea del log que el jugador pueda provocar.
- **Qué no he verificado:** que el `latest.log` de Minecraft 26.3 incluya líneas de chat con ese formato (el log de esta
  máquina no tenía líneas de LAN ni de chat para comprobarlo). En versiones 1.x el servidor integrado sí registra el chat.
- **Arreglo propuesto:** aceptar el marcador solo si la línea tiene el formato del propio servidor (prefijo
  `[hh:mm:ss] [Server thread/INFO]` seguido del mensaje, y rechazar las que lleven `<nombre>` o `[CHAT]` antes del
  marcador); y comprobar que el puerto pertenece al proceso Java de esa instancia. Test: el log del ejemplo anterior debe dar
  `None`.
- **Compatibilidad:** ninguna (lógica local; no cambia protocolo ni Worker).

### SEC-02 · Media · La privacidad de la cuenta no se aplica en el P2P
- **Dónde:** `accounts.rs:162-186` (`sync_account`) y `p2p.rs:446-486` (`Request::FriendRequest`).
- **Qué pasa:** el servidor aplica «amigos de amigos» y los bloqueos, pero el P2P es directo entre launchers. Al sincronizar,
  el cliente solo traslada `friend_privacy == "none"` y `invite_privacy == "none"` (`accounts.rs:174`); la lista de bloqueados
  de la cuenta (`social.blocked`) no se usa. Y cualquier lanzador puede mandar una `FriendRequest` P2P a un código que conozca.
- **Cómo se consigue el código:** `GET /users/search` devuelve `id` (= clave pública = código P2P) de cualquier usuario
  «buscable» (`core.mjs:195-205`), así que cualquier cuenta puede obtener el código de otra.
- **Escenario:** alguien al que bloqueaste en la cuenta, o un desconocido cuando tienes «solo amigos de amigos», te manda una
  solicitud P2P directa: aparece en tu lista como «entrante» con el nombre que él quiera. Ahí el nombre no es verificable
  (ver SEC-07).
- **Arreglo propuesto:** al sincronizar, (a) marcar como bloqueados en el P2P los de `social.blocked`; (b) tratar
  `friend_privacy == "mutual"` como «solo confiables»: aceptar solicitudes P2P de desconocidos únicamente si están en la lista
  de amigos de la cuenta.
- **Compatibilidad:** cambia lo que ve el usuario (algunas solicitudes P2P dejarían de llegar). **Necesito tu decisión.**

### SEC-03 · Media · El enlace profundo lanza el juego y entra en un servidor sin confirmar
- **Dónde:** `packages/app-lib/src/api/handler.rs:54-100` y `apps/app-frontend/src/App.vue:1725-1735`.
- **Qué pasa:** `strata-local://launch/instance/<id>?server=<host:puerto>` (también `modrinth://…`) arranca la instancia y se
  conecta al servidor indicado sin ningún cuadro de confirmación. El `<id>` es el nombre de la carpeta de la instancia (por
  ejemplo `Vanilla%2026.3`), fácil de adivinar.
- **Escenario:** una web maliciosa abre ese enlace; el navegador pide permiso para abrir Strata y, tras aceptarlo, el juego se
  inicia y entra en el servidor del atacante. Es comportamiento heredado de Modrinth, pero ahí el usuario no ve a qué servidor.
- **Arreglo propuesto:** pedir confirmación mostrando instancia y servidor cuando el comando viene de un enlace
  (`server` o `singleplayer_world` presentes).
- **Compatibilidad:** visible para el usuario. **Necesito tu decisión.**

### SEC-04 · Media · Descargas sin comprobar hash
- **Dónde:** `apps/app/src/api/local_servers.rs:949` (Purpur), `:985` (Fabric), `:989` (BungeeCord), `:994` (Forge, instalador),
  `:999` (NeoForge, instalador). Solo Paper (`:930`, SHA-256) y Vanilla (`:971`, SHA-1) se verifican.
- **Escenario:** HTTPS protege el transporte, pero un compromiso del servidor de descargas (o de un artefacto concreto) no se
  detecta. Los instaladores de Forge y NeoForge se ejecutan con Java.
- **Arreglo propuesto:** verificar con el hash que publica cada origen cuando existe (Forge/NeoForge publican `.sha1` junto al
  `.jar`; Purpur incluye el MD5 en el JSON de la compilación; Fabric y Jenkins de BungeeCord, no). Donde el origen no lo
  publique, quedará documentado como limitación.
- **Compatibilidad:** ninguna.

### SEC-05 · Media · Un amigo puede inflar disco y CPU con el chat
- **Dónde:** `p2p.rs:515-523` (`Request::Chat`) y `:370-384` (`with_chat`).
- **Qué pasa:** `id` y `ts` del mensaje no se validan; el marco admite hasta 64 KB, así que cada mensaje puede traer un `id` de
  ~64 KB. `with_chat` guarda hasta 1000 mensajes y **reescribe el archivo completo en cada mensaje**. Cada amigo aceptado puede
  así forzar ~64 MB por conversación en disco/memoria y escrituras de ese tamaño por cada mensaje que envíe. La
  deduplicación solo mira el `id`, que el emisor controla.
- **Arreglo propuesto:** exigir `id` de longitud ≤ 64 y con caracteres de UUID, rechazar `ts` fuera de un margen razonable y
  limitar la cantidad de mensajes por minuto por amigo. Test con un `id` de 64 KB.
- **Compatibilidad:** los launchers actuales envían UUID v4 (`p2p.rs:1003`), así que no se rompe a otras versiones de Strata.

### SEC-06 · Media · Worker: sondeo de usuarios sin límite
- **Dónde:** `core.mjs:211-233`.
- **Qué pasa:** en las acciones de amistad, el usuario destino se busca (`:212`) **antes** del límite `req:` (`:233`, solo para
  `/friends/request` y solo cuando el destino existe). `/unblock`, `/friends/remove` o `/block` contra un nombre inexistente
  responden 400 y contra uno existente `ok`, sin límite alguno. Eso permite comprobar si existe un nombre aunque el usuario
  haya desactivado «buscable», y probar nombres a ritmo ilimitado.
- **Arreglo propuesto:** aplicar un límite por cuenta a todas las acciones de `FRIEND_ACTIONS` antes de buscar al destino y
  devolver la misma respuesta exista o no el destino.
- **Compatibilidad:** no cambia el esquema D1 ni la API; el cliente ya trata 400/429.
  Cambia la respuesta cuando el destino no existe, así que lo tratamos como decisión tuya.

### SEC-07 · Baja · P2P: pares desconocidos sin límite y nombres suplantables
- **Dónde:** `p2p.rs:399-415` (`accept_loop`) y `:446-486`.
- **Qué pasa:** cualquiera que conozca tu código puede abrir conexiones y streams sin tope antes de ser amigo; cada stream
  espera hasta 12 s (`REQUEST_TIMEOUT`). `clean_name` quita caracteres de control pero no los de dirección de texto
  (U+202E…) ni los de ancho cero, y no hay nada que impida usar el nombre de un amigo real.
- **Arreglo propuesto:** tope de conexiones y de streams simultáneos por par y global; normalizar el nombre (quitar
  caracteres de formato Unicode) y mostrar siempre el código corto junto al nombre en las solicitudes.
- **Compatibilidad:** sin cambios de protocolo; el segundo punto es visible (solo en la pantalla de solicitudes).

### SEC-08 · Baja · ZIP bomb por tamaño declarado
- **Dónde:** `local_servers.rs:255-263` y `local_server_packs.rs:160-165`.
- **Qué pasa:** el tope de 20 GB / 5 GB se calcula con `entry.size()` (tamaño declarado en la cabecera) y luego se copia con
  `std::io::copy` sin acotar los bytes realmente descomprimidos. Un archivo que declare menos de lo que expande puede escribir
  mucho más. **No verificado con un archivo malicioso real**; depende del comportamiento del crate `zip` en esta versión.
- **Escenario:** el usuario importa un ZIP o `.mrpack` hostil y se llena el disco.
- **Arreglo propuesto:** copiar con `entry.take(declared + 1)` y fallar si se pasa del tamaño declarado. Test con una entrada
  que miente.
- **Compatibilidad:** ninguna.

### SEC-09 · Baja · Worker: repetición dentro de la ventana de 5 minutos
- **Dónde:** `core.mjs:46-60` (`authenticate`).
- **Qué pasa:** la firma cubre método, ruta, hora y hash del cuerpo, pero no hay nonce ni registro de firmas usadas: la misma
  petición vale durante `SKEW_MS` (5 min). Para aprovecharlo hay que capturarla, y viaja por TLS hacia Cloudflare, así que lo
  dejo en Baja.
- **Arreglo propuesto:** guardar el hash de la firma durante 5 min (tabla `limits` o nueva) y rechazar repeticiones.
- **Compatibilidad:** no cambia el formato; si se guardan firmas en D1 se añade una escritura por petición (mira la cuota). **Decisión tuya.**

### SEC-10 · Baja · Worker: sin límite general; riesgo de agotar la cuota
- **Dónde:** `core.mjs:46-70`, `:119-135`.
- **Qué pasa:** solo hay límites en registro (por IP), búsqueda y solicitudes de amistad. Una clave sin cuenta paga una
  verificación Ed25519 y una consulta D1 por petición sin tope; el plan gratuito de Cloudflare permite 100 000 peticiones al día,
  así que un flujo constante puede dejar el servicio sin cuota (DoS económico, no fuga de datos).
- **Arreglo propuesto:** una regla de «Rate limiting» de Cloudflare por IP (se configura en el panel, no en el código) y, en el
  código, devolver 401 antes de tocar D1 para claves sin cuenta. **Decisión tuya** (necesita tocar tu panel de Cloudflare).

### SEC-11 · Baja · Tuberías del audio con el control de acceso por defecto
- **Dónde:** `apps/app/src/api/recording/audio.rs:146`.
- **Qué pasa:** `\\.\pipe\strata-rec-<uuid>-juego|micro` se crean con `reject_remote_clients(true)` y
  `first_pipe_instance(true)` (bien), pero con el descriptor de seguridad por defecto, que da lectura a «Todos». Otro proceso
  local que enumere `\\.\pipe\` podría leer en vivo el audio del juego o el micrófono mientras se graba. El nombre lleva un UUID
  aleatorio, pero los nombres de tuberías son enumerables.
- **Arreglo propuesto:** crear la tubería con un DACL que solo permita al usuario actual (`CreateNamedPipeW` con
  `SECURITY_ATTRIBUTES`, ya que tokio no lo expone). Es código `unsafe` pequeño. Prueba: un segundo usuario/proceso con otro
  token no debe poder abrirla.
- **Compatibilidad:** ninguna.

### SEC-12 · Baja · Restos de configuración
- **Dónde:** `apps/app/capabilities/plugins.json:44-52`; `apps/app/tauri.conf.json:123-134`.
- **Qué pasa:** el permiso HTTP incluye `http://localhost:8000`, `http://*.taila228c5.ts.net` y `https://*.taila228c5.ts.net`
  (entorno de desarrollo de Modrinth): Strata no los usa y el primero va sin cifrar. La CSP no define `base-uri` ni
  `form-action` (no heredan de `default-src`) y permite `http://textures.minecraft.net` en claro (`connect-src`, `img-src`).
- **Arreglo propuesto:** quitar esas tres entradas de `plugins.json`; añadir `base-uri 'self'; form-action 'none'` a la CSP;
  probar que las skins siguen cargando por HTTPS (la CSP ya permite `https://textures.minecraft.net`).
- **Compatibilidad:** ninguna.

### SEC-13 · Baja · Límite de túneles no atómico
- **Dónde:** `p2p.rs:750-760` (comprobación) frente a `:732` (incremento).
- **Qué pasa:** entre comprobar `MAX_TUNNELS_PER_FRIEND` y sumar el túnel hay un `await`, así que una ráfaga de aperturas
  simultáneas del mismo amigo puede superar el límite.
- **Arreglo propuesto:** comprobar y reservar bajo el mismo bloqueo.
- **Compatibilidad:** ninguna.

### SEC-14 · Info · La clave de la cuenta se guarda sin cifrar
- **Dónde:** `p2p.rs:274-283` (`identity.key`, hexadecimal) y `:1297-1306` (`p2p_export_identity`).
- **Qué pasa:** la clave **es** la cuenta. Queda en `%APPDATA%\com.strata.minecraft.local\p2p\identity.key` con los permisos del
  perfil de usuario (suficiente frente a otros usuarios, no frente a malware que corra como tú). La exportación escribe una
  copia en la ruta que se elija (por ejemplo una carpeta sincronizada en la nube) sin avisar del riesgo.
- **Propuesta (opcional):** proteger el archivo con DPAPI y avisar al exportar. **Decisión tuya**: añade complejidad y cambia
  el formato de archivo.

### SEC-15 · Info · Dependencias
Método: `cargo audit` y `pnpm audit` no están instalados aquí. Usé la base pública OSV (que incluye RustSec y los avisos de
GitHub) sobre `Cargo.lock` y `pnpm-lock.yaml`, y filtré por lo que realmente entra en el launcher (`cargo tree -p theseus_gui`
para Windows; dependencias de producción de `apps/app-frontend`). El monorepo completo tiene 143 paquetes con avisos, pero
casi todos son de la web y los servidores de Modrinth, que Strata no incluye.

Alcanzan el launcher (ninguno encaja con un camino de ataque que haya podido confirmar):
- **Rust:** `h2 0.4.12` (RUSTSEC-2026-0258, tramas DATA vacías sin límite), `hickory-proto/resolver 0.24/0.25` (DNSSEC y CPU
  en el resolvedor DNS; no se usa validación DNSSEC), `quick-xml 0.38.3` (DoS por XML hostil), `anyhow`, `event-listener`,
  `crossbeam-epoch`, `rand` (solidez, no explotables), y crates sin mantenimiento (`fxhash`, `paste`, `serde_cbor`, `unic-*`).
- **npm (producción):** `markdown-it 13/14` y `linkify-it 4/5` (CPU cuadrática con texto hostil: una descripción de proyecto
  enorme podría congelar la interfaz), `nanoid`, `fflate`, `postcss` (avisos de herramientas de compilación), `dompurify
  3.3.1` (solo se usa en la capa de hosting de Modrinth, que Strata no carga: no aplica).
- **Propuesta:** actualizar `h2`, `hickory-*`, `quick-xml`, `markdown-it` y `linkify-it` dentro de rango compatible y
  recompilar. Cada actualización exige compilar y probar.

## No cubierto en profundidad

- El resto del núcleo heredado de Modrinth (`packages/app-lib`: descargas de Minecraft y loaders, autenticación de Microsoft,
  `instance.rs`, importadores de otros launchers). Solo muestreé la instalación de `.mrpack`, que usa
  `SafeRelativeUtf8UnixPathBuf` para las rutas, y el manejador de enlaces.
- Código de macOS y Linux.
- Pruebas dinámicas (fuzzing o ataques reales contra un par P2P o contra el Worker desplegado). Todo lo marcado «Por lectura»
  no se ha explotado de verdad.

## Notas sobre cómo quedó cada arreglo

- **SEC-01:** el log solo vale si lleva delante el prefijo del propio juego (`[hh:mm:ss] [hilo/NIVEL]:`, `[System]`,
  `[CHAT]`) y el puerto es ≥ 1024. Límite conocido: no se comprueba que el puerto pertenezca al proceso Java de la instancia.
  Test: `chat_lines_cannot_redirect_the_lan_port`. El código antiguo devolvía 3306 con esas líneas (lo comprobé antes de
  tocar nada).
- **SEC-02:** los bloqueados de la cuenta pasan a bloqueados del P2P (se levantan cuando la cuenta los levanta; un bloqueo
  manual no se toca) y «amigos de amigos» cierra las solicitudes de desconocidos igual que «nadie». Los amigos de la cuenta
  siguen entrando. Efecto visible: alguien que no sea amigo de la cuenta ya no podrá mandarte solicitudes P2P directas con
  esa privacidad.
- **SEC-03:** diálogo nativo de confirmación solo cuando el enlace trae `server` o `singleplayer_world`. Un enlace que solo
  abre la instancia sigue sin preguntar (heredado de Modrinth).
- **SEC-04:** los instaladores de Forge y NeoForge se verifican con el `.sha512`/`.sha256`/`.sha1` que publica su repositorio
  (comprobé en real que existen y que el hash publicado coincide con el instalador de Forge 1.20.1); si no se puede obtener,
  no se descarga. Un hash del mismo origen detecta corrupción y alteraciones parciales, no el compromiso completo del
  origen. Purpur, Fabric y BungeeCord no publican hash: **sin arreglar**.
- **SEC-05:** `id` ≤ 64 caracteres alfanuméricos o guiones (los launchers actuales envían UUID), hora acotada y 60
  mensajes por minuto y amigo. No rompe a otras versiones de Strata.
- **SEC-06:** 60 acciones por hora y cuenta, el límite de 20 solicitudes por hora antes de buscar al destino, y
  bloquear/desbloquear/retirar/aceptar responden igual exista o no el usuario. Una solicitud a un nombre inexistente sigue
  dando 400 (inherente a que el usuario sepa si su solicitud llegó); queda acotada por el límite. **Hay que redesplegar el
  Worker.** No cambia el esquema D1 ni la API.
- **SEC-07:** 256 conexiones entrantes simultáneas y 64 streams por conexión; nombres sin caracteres invisibles de formato.
  Además, `save_store` usaba el mismo `friends.json.tmp` en todos los guardados: un test del P2P falló una vez bajo carga
  y con el arreglo pasaron 3 de 3 suites completas. No logré reproducir el fallo de forma determinista, así que no puedo
  afirmar que esa fuera la causa.
- **SEC-08:** probado con un ZIP fabricado (declara 1000 bytes y descomprime 10 MB sin error en el crate `zip`). La copia
  se acota al tamaño declarado en la importación de ZIP, en los packs de servidor y en el índice del modpack.
- **SEC-11:** DACL `D:P(A;;GA;;;OW)(A;;GA;;;SY)`. El test lee la DACL de una tubería por defecto (da acceso a «Todos»)
  y de la nuestra (no) y comprueba que el propio usuario puede conectar.
- **SEC-12:** quitados `localhost:8000`, `127.0.0.1:8000` y `*.taila228c5.ts.net`; la CSP gana `base-uri 'self'` y
  `form-action 'none'`. Se mantiene `http://textures.minecraft.net` porque las skins se sirven así.
- **SEC-13:** el hueco del túnel se reserva al autorizar, bajo el mismo bloqueo.

## Sin hacer, y por qué

- **SEC-09 (repetición de firmas):** no lo aplicaste. Añadiría una escritura a D1 por petición.
- **SEC-10 (cuota del Worker):** la solución principal es una regla de «Rate limiting» en el panel de Cloudflare.
- **SEC-14 (clave de cuenta sin cifrar):** requiere cambiar el formato del archivo y tu decisión.
- **SEC-15 (dependencias):** no se ha actualizado ninguna; cada una exige compilar y probar. Candidatas: `h2`, `hickory-*`,
  `quick-xml`, `markdown-it`, `linkify-it`.
- **No cubierto en profundidad:** el núcleo de Modrinth (`packages/app-lib`), macOS y Linux, y pruebas dinámicas contra
  el Worker desplegado.

## Qué deberías probar tú a mano

1. **Redesplegar el Worker** (`scripts/deploy-accounts.ps1`) y comprobar que registrarte, buscar y mandar una solicitud de
   amistad siguen funcionando.
2. **Enlace profundo:** abre `strata-local://launch/instance/<tu instancia>?server=localhost:25565` desde el navegador o
   con `start`; debe salir el diálogo con la instancia y el servidor y no lanzarse si cancelas.
3. **Mundo LAN compartido:** abre un mundo a LAN, compártelo con un amigo y comprueba que sigue pudiendo entrar. Escribe en
   el chat «Started serving on 3306»: el túnel debe seguir yendo al puerto real.
4. **Bloqueo en la cuenta:** bloquea a un usuario de prueba en la cuenta y comprueba que sale como bloqueado en Amigos y
   que no puede mandarte una solicitud P2P.
5. **Grabación con audio y micrófono** (las tuberías cambian de permisos): graba 20 s y comprueba que el vídeo tiene audio.
6. **Instalar un servidor Forge o NeoForge** desde Servidores y ver que descarga e instala.
7. **Skins y la interfaz en general** tras el cambio de CSP: abre Skins y una página de proyecto y mira que carga todo.
