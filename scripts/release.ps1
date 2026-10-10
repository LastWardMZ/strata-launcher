<#
.SYNOPSIS
  Compila, firma y publica una versión de Strata para que los launchers instalados se actualicen solos.

.EXAMPLE
  .\scripts\release.ps1 -Version 0.2.1 -Notes "Arreglos del chat"            # solo compila y prepara los archivos
  .\scripts\release.ps1 -Version 0.2.1 -Notes "Arreglos del chat" -Publish   # además crea la Release en GitHub

Requisitos: la clave de firma en $env:USERPROFILE\.tauri\strata-updater.key (NO se sube nunca al repositorio;
si se pierde, las instalaciones existentes ya no podrán actualizarse) y `gh` con sesión iniciada para -Publish.
#>
param(
	[Parameter(Mandatory = $true)][string]$Version,
	[string]$Notes = 'Mejoras y correcciones.',
	[switch]$Publish,
	[switch]$Rebuild,
	[string]$Repo = 'LastWardMZ/strata-launcher'
)

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Versión no válida '$Version'. Usa el formato 1.2.3" }

$root = Split-Path -Parent $PSScriptRoot
$app = Join-Path $root 'code-main\apps\app-frontend\package.json'
$key = Join-Path $env:USERPROFILE '.tauri\strata-updater.key'
$entrega = Join-Path $root 'Entrega'
if (-not (Test-Path $key)) { throw "Falta la clave de firma: $key" }
if (Get-Process Strata -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$root*" }) {
	throw 'Cierra Strata antes de compilar: bloquea el ejecutable que hay que sustituir.'
}

# 1. Versión (la fuente de verdad es el package.json del frontend)
$json = Get-Content $app -Raw
$current = [regex]::Match($json, '"version":\s*"([^"]+)"').Groups[1].Value
if ([version]$Version -lt [version]$current -or ([version]$Version -eq [version]$current -and -not $Rebuild)) { throw "La versión $Version debe ser mayor que la actual ($current): si no, los clientes no la verán como actualización (usa -Rebuild para recompilar la misma versión sin publicarla)." }
# UTF-8 SIN BOM: Tauri no puede leer un package.json con BOM.
$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($app, [regex]::Replace($json, '"version":\s*"[^"]+"', "`"version`": `"$Version`"", 1), $utf8)
Write-Host "Versión $current -> $Version"

# 2. Entorno de compilación aislado de este equipo
$env:CARGO_HOME = Join-Path $root '.cargo-home'
$env:RUSTUP_HOME = Join-Path $root '.rustup-home'
$env:CARGO_TARGET_DIR = Join-Path $root 'target'
$env:PATH = "$(Join-Path $root '.cargo-home\bin');$(Join-Path $root '.build-tools');$env:PATH"
$env:CI = 'true'
Get-Content (Join-Path $root 'code-main\packages\app-lib\.env.prod') | ForEach-Object {
	if ($_ -match '^\s*([A-Z_0-9]+)=(.*)$') { Set-Item "env:$($Matches[1])" $Matches[2].Trim('"') }
}
$env:TAURI_SIGNING_PRIVATE_KEY = $key

# 3. Compilar firmado y con el actualizador activado
# cargo escribe su progreso por stderr: con 'Stop', PowerShell 5.1 lo trataría como un error.
$ErrorActionPreference = 'Continue'
Push-Location (Join-Path $root 'code-main')
try {
	pnpm --filter '@modrinth/app' build --features updater
	$buildExit = $LASTEXITCODE
} finally { Pop-Location }
$ErrorActionPreference = 'Stop'
if ($buildExit -ne 0) { throw "La compilación falló (código $buildExit)" }

# 4. Archivos de la versión
$bundle = Join-Path $root 'target\release\bundle\nsis'
$setup = Join-Path $bundle "Strata Minecraft_${Version}_x64-setup.exe"
$sig = "$setup.sig"
if (-not (Test-Path $setup)) { throw "No se generó el instalador: $setup" }
if (-not (Test-Path $sig)) { throw "No se generó la firma: $sig (¿createUpdaterArtifacts?)" }
New-Item -ItemType Directory -Force $entrega | Out-Null
$asset = "Strata-Setup-$Version.exe"
Copy-Item $setup (Join-Path $entrega $asset) -Force
$latest = [ordered]@{
	version = $Version
	notes = $Notes
	pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
	platforms = [ordered]@{
		'windows-x86_64' = [ordered]@{
			signature = (Get-Content $sig -Raw).Trim()
			url = "https://github.com/$Repo/releases/download/v$Version/$asset"
		}
	}
}
$latestPath = Join-Path $entrega 'latest.json'
[System.IO.File]::WriteAllText($latestPath, ($latest | ConvertTo-Json -Depth 5), $utf8)
Write-Host "Listo en ${entrega}: $asset y latest.json"
Write-Host ('SHA-256: ' + (Get-FileHash (Join-Path $entrega $asset) -Algorithm SHA256).Hash)

# 5. Publicar
if ($Publish) {
	gh release create "v$Version" (Join-Path $entrega $asset) $latestPath --repo $Repo --title "Strata $Version" --notes $Notes
	if ($LASTEXITCODE -ne 0) { throw 'No se pudo crear la Release en GitHub' }
	Write-Host "Publicada: https://github.com/$Repo/releases/tag/v$Version"
} else {
	Write-Host 'No se ha publicado (falta -Publish).'
}
