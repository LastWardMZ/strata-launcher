/**
 * Enlace entre los amigos de Modrinth y el P2P de Strata.
 *
 * Modrinth gestiona quién es amigo de quién, pero no sabe nada del P2P. Para que dos amigos puedan
 * conectarse sin intercambiar códigos a mano, cada uno puede publicar su código de partidas (la
 * clave pública de 32 bytes) en su biografía de Modrinth con un marcador `strata:<base64url>`.
 * La biografía de Modrinth admite como máximo 160 caracteres, y el marcador ocupa 50.
 */
export const BIO_MAX_LENGTH = 160
const MARKER_PREFIX = 'strata:'
const MARKER_PATTERN = /strata:([A-Za-z0-9_-]{43})(?![A-Za-z0-9_-])/g

/** Código de partidas (64 caracteres hexadecimales) → marcador `strata:…` de 50 caracteres. */
export function codeToMarker(code: string): string {
	if (!/^[0-9a-f]{64}$/i.test(code)) throw new Error('Código de partidas no válido')
	let binary = ''
	for (let i = 0; i < 64; i += 2) binary += String.fromCharCode(parseInt(code.slice(i, i + 2), 16))
	const base64url = btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
	return MARKER_PREFIX + base64url
}

function markerPayloadToCode(payload: string): string | null {
	try {
		const base64 = payload.replace(/-/g, '+').replace(/_/g, '/') + '='
		const binary = atob(base64)
		if (binary.length !== 32) return null
		return Array.from(binary, (char) => char.charCodeAt(0).toString(16).padStart(2, '0')).join('')
	} catch {
		return null
	}
}

/** Primer código de partidas válido publicado en un texto (por ejemplo, una biografía), o `null`. */
export function codeFromBio(bio: string | null | undefined): string | null {
	if (!bio) return null
	for (const match of bio.matchAll(MARKER_PATTERN)) {
		const code = markerPayloadToCode(match[1])
		if (code) return code
	}
	return null
}

/** Biografía sin ningún marcador de Strata (conserva el resto del texto). */
export function bioWithoutMarker(bio: string | null | undefined): string {
	return (bio ?? '')
		.replace(MARKER_PATTERN, '')
		.replace(/[ \t]{2,}/g, ' ')
		.trim()
}

/**
 * Biografía con el marcador del código dado, sustituyendo cualquier marcador anterior.
 * Devuelve `null` si el resultado superaría el límite de Modrinth.
 */
export function bioWithMarker(bio: string | null | undefined, code: string): string | null {
	const base = bioWithoutMarker(bio)
	const marker = codeToMarker(code)
	const next = base ? `${base} ${marker}` : marker
	return next.length <= BIO_MAX_LENGTH ? next : null
}
