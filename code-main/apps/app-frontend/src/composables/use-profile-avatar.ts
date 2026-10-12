import { ref } from 'vue'

/**
 * Foto de perfil de este equipo. Se recorta al centro, se reduce a 128×128 y se recomprime a WebP
 * (unos pocos KB) para poder enviarla más adelante sin cargar el servicio de cuentas.
 */
const KEY = 'strata:profile-avatar'
const SIDE = 128
const MAX_BYTES = 24 * 1024
const MAX_SOURCE_BYTES = 10 * 1024 * 1024

const read = () => {
	try {
		return localStorage.getItem(KEY)
	} catch {
		return null
	}
}
const avatar = ref<string | null>(read())

function persist(value: string | null) {
	avatar.value = value
	try {
		if (value) localStorage.setItem(KEY, value)
		else localStorage.removeItem(KEY)
	} catch {
		// Sin almacenamiento la foto dura mientras la app esté abierta.
	}
}

const toBlob = (canvas: HTMLCanvasElement, quality: number) =>
	new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/webp', quality))

const toDataUrl = (blob: Blob) =>
	new Promise<string>((resolve, reject) => {
		const reader = new FileReader()
		reader.onload = () => resolve(String(reader.result))
		reader.onerror = () => reject(reader.error)
		reader.readAsDataURL(blob)
	})

export function useProfileAvatar() {
	/** Procesa la imagen elegida y devuelve el WebP en base64 (sin prefijo); lanza un `Error` legible si no sirve. */
	async function setFromFile(file: File): Promise<string> {
		if (!file.type.startsWith('image/')) throw new Error('Elige un archivo de imagen (PNG, JPG o WebP).')
		if (file.size > MAX_SOURCE_BYTES) throw new Error('La imagen pesa más de 10 MB.')
		let bitmap: ImageBitmap
		try {
			bitmap = await createImageBitmap(file)
		} catch {
			throw new Error('No se pudo leer la imagen.')
		}
		const side = Math.min(bitmap.width, bitmap.height)
		const canvas = document.createElement('canvas')
		canvas.width = canvas.height = SIDE
		const context = canvas.getContext('2d')
		if (!context) throw new Error('No se pudo procesar la imagen.')
		context.imageSmoothingQuality = 'high'
		context.drawImage(
			bitmap,
			(bitmap.width - side) / 2,
			(bitmap.height - side) / 2,
			side,
			side,
			0,
			0,
			SIDE,
			SIDE,
		)
		bitmap.close()
		for (const quality of [0.9, 0.8, 0.7, 0.55, 0.4]) {
			const blob = await toBlob(canvas, quality)
			if (blob && blob.size <= MAX_BYTES) {
				const dataUrl = await toDataUrl(blob)
				persist(dataUrl)
				return dataUrl.slice(dataUrl.indexOf(',') + 1)
			}
		}
		throw new Error('No se pudo reducir la imagen lo suficiente. Prueba con otra.')
	}
	return { avatar, setFromFile, clear: () => persist(null) }
}
