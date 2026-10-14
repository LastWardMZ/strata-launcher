import { createApp } from 'vue'

import { initAccentColor } from '@/helpers/accent-color'
import OverlayApp from '@/overlay/OverlayApp.vue'

export function mountOverlay() {
	// Encima del juego siempre oscuro y con el fondo transparente: el panel pone su propio velo.
	document.documentElement.classList.add('dark-mode')
	document.documentElement.style.background = 'transparent'
	document.body.style.background = 'transparent'
	initAccentColor()
	createApp(OverlayApp).mount('#app')
}
