import xss from 'xss'

// @ts-expect-error xss types don't reflect CJS default export shape
const { FilterXSS, escapeAttrValue } = xss

/**
 * Descripciones HTML de fuentes externas (p. ej. CurseForge). Es más estricto que `renderString`, pensado para Markdown de
 * Modrinth: aquí no hay scripts, iframes, estilos, clases ni eventos, y las imágenes solo se cargan del CDN de CurseForge
 * (nada de rastreadores ni de pasar por servicios de terceros). Los enlaces siempre se abren fuera y sin enviar el origen.
 */
const IMAGE_HOST = 'forgecdn.net'
const MAX_LENGTH = 200_000

const heading = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6']
const plain = ['p', 'br', 'b', 'strong', 'i', 'em', 'u', 's', 'strike', 'del', 'ul', 'ol', 'li', 'blockquote', 'code', 'pre', 'hr', 'sub', 'sup', 'span', 'div', 'table', 'thead', 'tbody', 'tr', 'td', 'th']

function parseUrl(value: string): URL | null {
	try {
		return new URL(value.trim().replaceAll('&amp;', '&'))
	} catch {
		return null
	}
}

const filter = new FilterXSS({
	whiteList: {
		...Object.fromEntries([...heading, ...plain].map((tag) => [tag, [] as string[]])),
		a: ['href'],
		img: ['src', 'alt', 'width', 'height'],
	},
	stripIgnoreTag: true,
	stripIgnoreTagBody: ['script', 'style', 'iframe', 'object', 'embed', 'form', 'input', 'button', 'textarea', 'select', 'svg', 'math', 'link', 'meta', 'base', 'noscript', 'template', 'audio', 'video'],
	onTagAttr(tag, name, value) {
		if (tag === 'a' && name === 'href') {
			const url = parseUrl(value)
			return url && ['https:', 'http:', 'mailto:'].includes(url.protocol) ? `href="${escapeAttrValue(url.toString())}"` : ''
		}
		if (tag === 'img' && name === 'src') {
			const url = parseUrl(value)
			const fromCdn = url && url.protocol === 'https:' && (url.hostname === IMAGE_HOST || url.hostname.endsWith(`.${IMAGE_HOST}`))
			return fromCdn ? `src="${escapeAttrValue(url.toString())}"` : ''
		}
		if (tag === 'img' && (name === 'width' || name === 'height')) {
			return /^\d{1,4}$/.test(value.trim()) ? `${name}="${value.trim()}"` : ''
		}
		return undefined
	},
})

export function renderExternalHtml(html: string): string {
	const clean = filter.process(String(html ?? '').slice(0, MAX_LENGTH))
	// El filtro ya deja los enlaces solo con `href`: se añade cómo abrirlos.
	return clean.replaceAll('<a href="', '<a rel="noopener nofollow ugc" target="_blank" href="')
}
