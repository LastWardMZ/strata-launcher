# Cuentas de Strata: prueba local

Servicio propio, independiente del proveedor del catálogo y de Microsoft/Minecraft. Requiere Node.js 24 o posterior con `node:sqlite`. No necesita paquetes externos.

Desde `code-main`:

```powershell
$env:STRATA_ACCOUNTS_DB='C:\ruta\de\datos\strata-accounts.sqlite'
node apps/strata-accounts/server.mjs
```

La aplicación de escritorio conecta exclusivamente con `http://127.0.0.1:32145`. El servicio escucha únicamente en la interfaz local. No se inicia automáticamente al instalar el launcher y no permite acceso desde otros ordenadores. Si no está arrancado, la interfaz muestra un error de servicio no disponible.

## Funciones

- Registro con nombre único, sin distinguir mayúsculas, y contraseña de 12–128 caracteres.
- Inicio de sesión y cierre de sesión. Tokens aleatorios de 256 bits, caducidad de 24 horas y revocación al salir. La aplicación guarda su token solo en memoria: al cerrar Strata hay que volver a entrar.
- Biografía persistente.
- Solicitudes recibidas/enviadas, aceptación, rechazo, cancelación y eliminación de amistades.
- Bloqueo y desbloqueo. Bloquear elimina la relación y evita nuevas solicitudes en ambos sentidos.
- Separación entre usuarios: todas las operaciones utilizan la identidad autenticada, nunca un identificador de propietario enviado por el cliente.

SQLite conserva perfiles, relaciones y sesiones. Las contraseñas se derivan con scrypt y una sal aleatoria individual; solo se guarda el hash del token. Las respuestas excluyen contraseña y sal. Los orígenes del navegador están limitados a Tauri y al desarrollo local, las solicitudes tienen límite de tamaño y los intentos de autenticación tienen límite por minuto.

## Comprobación

```powershell
node --test apps/strata-accounts/server.test.mjs
```

La prueba usa una base temporal independiente y comprueba autenticación inválida, aislamiento de perfiles, solicitudes, aceptación, bloqueo, cancelación, logout, orígenes no autorizados y persistencia tras reiniciar el servicio.

## Trabajo pendiente para usuarios reales

Esta es una primera fase local. No se ha desplegado un servicio público. Faltan configuración de dominio y HTTPS, operación y copias del servicio, recuperación de cuenta, avatar y edición de nombre, presencia en tiempo real e invitaciones para jugar. El instalador del launcher no incorpora Node.js ni este servicio como servicio de Windows. Las funciones de cuenta del proveedor del catálogo, como instancias compartidas de ese proveedor, mantienen su autenticación separada y no aceptan tokens de Strata.

No apuntar el cliente a un servicio remoto por HTTP ni cambiar la escucha a una dirección pública para distribuir esta prueba.
