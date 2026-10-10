<#
.SYNOPSIS
  Despliega el servicio de cuentas de Strata en Cloudflare Workers + D1 (plan gratuito, sin servidor propio).

.DESCRIPTION
  Hace, en este orden: instala wrangler (la herramienta de Cloudflare) en una carpeta aparte, inicia sesión en
  tu cuenta de Cloudflare (se abre el navegador), crea la base de datos D1, aplica el esquema y publica el
  servicio. Al final muestra la URL. Es idempotente: se puede volver a ejecutar para actualizar el servicio.

  Necesitas Node.js y una cuenta de Cloudflare (gratuita, sin tarjeta). Nada de esto se ejecuta solo: solo
  cuando lanzas este script.

.EXAMPLE
  .\scripts\deploy-accounts.ps1
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$service = Join-Path $root 'code-main\apps\strata-accounts'
$config = Join-Path $service 'wrangler.toml'
$tools = Join-Path $env:LOCALAPPDATA 'Strata\wrangler'
$wrangler = Join-Path $tools 'node_modules\wrangler\bin\wrangler.js'
$databaseName = 'strata-accounts'

if (-not (Get-Command node -ErrorAction SilentlyContinue)) { throw 'Falta Node.js. Instálalo desde https://nodejs.org y vuelve a ejecutar este script.' }

function Invoke-Wrangler {
	# Salida directa a la consola (necesaria para el inicio de sesión y las preguntas interactivas).
	param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
	& node $wrangler @Arguments --config $config
	if ($LASTEXITCODE -ne 0) { throw "wrangler ha fallado (código $LASTEXITCODE): $($Arguments -join ' ')" }
}
function Get-WranglerOutput {
	# Captura la salida (no interactivo); se usa solo cuando hace falta leer un dato.
	param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
	$previous = $ErrorActionPreference
	$ErrorActionPreference = 'Continue'
	try { $text = (& node $wrangler @Arguments --config $config 2>&1 | ForEach-Object { "$_" }) -join "`n" } finally { $ErrorActionPreference = $previous }
	return [pscustomobject]@{ Text = $text; Code = $LASTEXITCODE }
}

# 1. Herramienta de Cloudflare
if (-not (Test-Path $wrangler)) {
	Write-Host '1/5 Instalando wrangler (una sola vez)…'
	New-Item -ItemType Directory -Force $tools | Out-Null
	& npm install wrangler@4 --prefix $tools --no-audit --no-fund
	if ($LASTEXITCODE -ne 0 -or -not (Test-Path $wrangler)) { throw 'No se pudo instalar wrangler.' }
} else { Write-Host '1/5 wrangler ya está instalado.' }

# 2. Sesión de Cloudflare
$who = Get-WranglerOutput whoami
if ($who.Text -notmatch 'You are logged in') {
	Write-Host '2/5 Inicia sesión en Cloudflare (se abrirá el navegador)…'
	Invoke-Wrangler login
} else { Write-Host '2/5 Ya tienes la sesión de Cloudflare iniciada.' }

# 3. Base de datos D1
$toml = Get-Content $config -Raw
if ($toml -match 'database_id\s*=\s*"00000000-0000-0000-0000-000000000000"') {
	Write-Host "3/5 Creando la base de datos '$databaseName'…"
	$created = Get-WranglerOutput d1 create $databaseName
	$id = [regex]::Match($created.Text, 'database_id\s*[=:]\s*"?([0-9a-f]{8}-[0-9a-f-]{27})').Groups[1].Value
	if (-not $id) {
		# Si ya existía, se busca en la lista.
		$list = Get-WranglerOutput d1 list --json
		$found = $null
		try { $found = ($list.Text | ConvertFrom-Json) | Where-Object { $_.name -eq $databaseName } | Select-Object -First 1 } catch {}
		if ($found) { $id = $found.uuid }
	}
	if (-not $id) { throw "No se pudo obtener el identificador de la base de datos.`n$($created.Text)" }
	$utf8 = New-Object System.Text.UTF8Encoding($false)
	[System.IO.File]::WriteAllText($config, ($toml -replace '00000000-0000-0000-0000-000000000000', $id), $utf8)
	Write-Host "     Base de datos creada: $id"
} else { Write-Host '3/5 La base de datos ya está configurada.' }

# 4. Esquema
Write-Host '4/5 Aplicando el esquema de la base de datos…'
Invoke-Wrangler d1 migrations apply $databaseName --remote

# 5. Publicar. La primera vez, Cloudflare puede pedir que elijas un subdominio gratuito (workers.dev).
Write-Host '5/5 Publicando el servicio…'
Invoke-Wrangler deploy
$deployed = Get-WranglerOutput deploy
$url = [regex]::Match($deployed.Text, 'https://[a-z0-9.-]+\.workers\.dev').Value
if (-not $url) { Write-Warning 'No se pudo leer la URL; mírala en https://dash.cloudflare.com (Workers & Pages > strata-accounts).' }

Write-Host ''
Write-Host 'Servicio de cuentas publicado.' -ForegroundColor Green
if ($url) {
	Write-Host "  URL: $url"
	Write-Host "  Comprobación: $url/health  (debe responder {""ok"":true})"
	Write-Host ''
	Write-Host 'Siguiente paso: publicar una versión de Strata que use ese servidor:'
	Write-Host "  .\scripts\release.ps1 -Version <versión> -AccountsUrl $url -Publish"
}
