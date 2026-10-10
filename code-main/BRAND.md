# Identidad visual de Strata

El símbolo es una S angular construida como una cinta de estratos. Su silueta y sus caras distinguen la marca a tamaños pequeños. La versión de aplicación utiliza una base oscura con esquinas transparentes, un borde discreto y caras en menta/turquesa.

## Archivos fuente

- `apps/app-frontend/public/brand/strata-icon.svg`: icono de aplicación, con base.
- `apps/app-frontend/public/brand/strata-mark.svg`: símbolo sin base para interfaces oscuras.
- `apps/app-frontend/public/brand/strata-mark-light.svg`: símbolo con contraste para interfaces claras.
- `apps/app-frontend/public/brand/strata-mark-mono.svg`: silueta blanca para fondos oscuros.
- `apps/app-frontend/src/components/ui/StrataBrand.vue`: símbolo y nombre reutilizables; adapta el símbolo al tema.

Todos los dibujos son recursos vectoriales creados para este proyecto. No dependen de imágenes alojadas en un servicio externo ni de elementos de marca de Modrinth o Minecraft.

## Aplicación

La cabecera, la bienvenida, la pantalla de carga y el pie de Ajustes usan la misma identidad. Los componentes compartidos TextLogo y AnimatedLogo utilizan la misma silueta. La carga respeta la preferencia del sistema de reducir movimiento. Los iconos de Windows, macOS y los tamaños generados por Tauri proceden del mismo SVG. El instalador NSIS usa un icono, una cabecera y una columna lateral propios.

La carpeta `../Brand` contiene un PNG de 1024 píxeles, el ICO de Windows, los originales SVG y una lámina con los tamaños pequeños. No estirar ni alterar la proporción del símbolo; usar la variante clara sobre fondos claros y la blanca sobre fondos oscuros. El icono con base funciona en ambos.

## Regeneración

Tras instalar las dependencias del monorepo:

```powershell
node scripts/build-strata-brand.mjs
```

El script utiliza el CLI de Tauri y sharp (dependencia existente de `apps/docs`). Regenera los tamaños de icono, el favicon, los gráficos BMP de NSIS y los archivos de exportación. El antiguo punto de entrada `scripts/rebrand-strata.py` delega en este generador para no restaurar el diseño anterior.

Para incluir los recursos nuevos en un instalador, compilar la aplicación como se describe en `../INTEGRACION.md`.

## Verificación

Se revisó el renderizado a tamaños pequeños. El ICO contiene 16, 24, 32, 48, 64 y 256 píxeles; el PNG principal conserva alfa transparente en las esquinas. Los gráficos del instalador miden 150×57 y 164×314 píxeles y son BMP de 24 bits. El build del frontend incluye la comprobación de tipos. La instalación en Windows y las plataformas distintas de Windows requieren sus propias comprobaciones; generar sus iconos no equivale a probar esas aplicaciones.
