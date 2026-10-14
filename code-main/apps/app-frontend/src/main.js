// La ventana del panel de amigos sobre el juego (Shift+Tab) carga una aplicación mínima: no necesita el router, el estado
// del launcher ni el resto de la interfaz.
if (new URLSearchParams(window.location.search).has('overlay')) {
	void import('@/overlay/mount').then(({ mountOverlay }) => mountOverlay())
} else {
	void import('@/main-app')
}
