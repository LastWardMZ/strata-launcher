import { ref } from 'vue'

/**
 * Color de acento del launcher. El usuario elige un color y aquí se traduce a las variables
 * CSS de marca (`--color-brand`…) con las que ya se pintan botones, enlaces y resaltados.
 * Se aplica como estilo en `<html>`, así que se impone al tema sin tocar sus archivos y
 * se recalcula al cambiar entre tema claro y oscuro para mantener el contraste.
 */
export type AccentPreset = { id: string; name: string; color: string }

export const ACCENT_PRESETS: AccentPreset[] = [
	{ id: 'strata', name: 'Azul Strata', color: '#1db1c3' },
	{ id: 'azure', name: 'Azul', color: '#4f9cff' },
	{ id: 'violet', name: 'Violeta', color: '#9b6dff' },
	{ id: 'pink', name: 'Rosa', color: '#f45e9a' },
	{ id: 'red', name: 'Rojo', color: '#ff4d5e' },
	{ id: 'orange', name: 'Naranja', color: '#ff9b3d' },
	{ id: 'yellow', name: 'Amarillo', color: '#f2c230' },
	{ id: 'green', name: 'Verde', color: '#1bd96a' },
]
export const DEFAULT_ACCENT = ACCENT_PRESETS[0].color

const STORAGE_KEY = 'strata:accent-color'
const HEX = /^#[0-9a-f]{6}$/i

type Rgb = [number, number, number]

function hexToRgb(hex: string): Rgb {
	return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb
}
function rgbToHex([r, g, b]: Rgb): string {
	return '#' + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')
}
function rgbToHsl([r, g, b]: Rgb): [number, number, number] {
	const [rn, gn, bn] = [r / 255, g / 255, b / 255]
	const max = Math.max(rn, gn, bn)
	const min = Math.min(rn, gn, bn)
	const l = (max + min) / 2
	if (max === min) return [0, 0, l]
	const d = max - min
	const s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
	let h = 0
	if (max === rn) h = (gn - bn) / d + (gn < bn ? 6 : 0)
	else if (max === gn) h = (bn - rn) / d + 2
	else h = (rn - gn) / d + 4
	return [h * 60, s, l]
}
function hslToRgb([h, s, l]: [number, number, number]): Rgb {
	if (s === 0) return [l * 255, l * 255, l * 255]
	const q = l < 0.5 ? l * (1 + s) : l + s - l * s
	const p = 2 * l - q
	const channel = (t: number) => {
		const x = (t + 360) % 360
		if (x < 60) return p + (q - p) * (x / 60)
		if (x < 180) return q
		if (x < 240) return p + (q - p) * ((240 - x) / 60)
		return p
	}
	return [channel(h + 120) * 255, channel(h) * 255, channel(h - 120) * 255]
}
function luminance([r, g, b]: Rgb): number {
	const lin = (v: number) => {
		const c = v / 255
		return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
	}
	return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}
const rgba = ([r, g, b]: Rgb, alpha: number) =>
	`rgba(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)}, ${alpha})`

/** Ajusta la luminosidad del color elegido al tema para que siga siendo legible. */
function fitToTheme(hex: string, light: boolean): Rgb {
	const [h, s, l] = rgbToHsl(hexToRgb(hex))
	if (!light) return hslToRgb([h, s, Math.min(Math.max(l, 0.45), 0.72)])
	// Tema claro: se oscurece solo lo necesario para que el texto blanco sobre el acento se lea.
	let fitted = Math.min(Math.max(l, 0.28), 0.55)
	while (fitted > 0.28 && 1.05 / (luminance(hslToRgb([h, s, fitted])) + 0.05) < 3.5) fitted -= 0.02
	return hslToRgb([h, s, fitted])
}

/** Variables CSS de marca derivadas de un color y del tema (claro u oscuro). */
export function accentVariables(hex: string, light: boolean): Record<string, string> {
	const rgb = fitToTheme(hex, light)
	const brand = rgbToHex(rgb)
	const [h, s, l] = rgbToHsl(rgb)
	const bright = rgbToHex(hslToRgb([h, s, Math.min(l + 0.12, 0.85)]))
	const onBlack = (luminance(rgb) + 0.05) / 0.05
	const onWhite = 1.05 / (luminance(rgb) + 0.05)
	return {
		'--color-brand': brand,
		'--color-brand-highlight': rgba(rgb, 0.25),
		'--color-brand-shadow': rgba(rgb, 0.7),
		'--color-accent-contrast': onBlack >= onWhite ? '#000000' : '#ffffff',
		'--brand-gradient-bg': `linear-gradient(0deg, ${rgba(rgb, light ? 0.175 : 0.12)} 0%, ${rgba(rgb, light ? 0.125 : 0.06)} 100%)`,
		'--brand-gradient-strong-bg': `linear-gradient(270deg, ${rgba(rgb, light ? 0.175 : 0.08)} 0%, ${rgba(rgb, light ? 0.12 : 0.16)} 100%)`,
		'--brand-gradient-border': rgba(rgb, light ? 0.15 : 0.12),
		'--loading-bar-gradient': `linear-gradient(to right, ${brand} 0%, ${bright} 100%)`,
		'--splash-tint-top': rgba(rgb, 0.275),
	}
}

function readStored(): string {
	try {
		const value = localStorage.getItem(STORAGE_KEY)
		if (value && HEX.test(value)) return value.toLowerCase()
	} catch {
		// Sin almacenamiento disponible: se usa el color por defecto.
	}
	return DEFAULT_ACCENT
}

export const accentColor = ref(DEFAULT_ACCENT)
let started = false

function isLightTheme(): boolean {
	return document.documentElement.classList.contains('light-mode')
}

export function applyAccent(): void {
	const root = document.documentElement
	for (const [name, value] of Object.entries(accentVariables(accentColor.value, isLightTheme()))) {
		root.style.setProperty(name, value)
	}
}

export function setAccentColor(hex: string): boolean {
	if (!HEX.test(hex)) return false
	accentColor.value = hex.toLowerCase()
	try {
		localStorage.setItem(STORAGE_KEY, accentColor.value)
	} catch {
		// El color sigue aplicándose en esta sesión aunque no se pueda guardar.
	}
	applyAccent()
	return true
}

export function resetAccentColor(): void {
	setAccentColor(DEFAULT_ACCENT)
}

/** Carga el color guardado, lo aplica y lo mantiene al cambiar de tema. Llamar una vez al arrancar. */
export function initAccentColor(): void {
	if (started) return
	started = true
	accentColor.value = readStored()
	applyAccent()
	new MutationObserver(applyAccent).observe(document.documentElement, {
		attributes: true,
		attributeFilter: ['class'],
	})
}
