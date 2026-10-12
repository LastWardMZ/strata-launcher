import { useQueryClient } from '@tanstack/vue-query'
import { onBeforeUnmount, onMounted } from 'vue'

import { formatClock, onRecordingEvent, videoKeys } from '@/helpers/recording'

type Notify = (notification: Record<string, unknown>) => unknown

/**
 * Avisos de la grabación, visibles en cualquier pantalla del launcher: empieza, se guarda, falla o falta el
 * componente. También mantiene al día la biblioteca de vídeos cuando termina una grabación.
 */
export function useRecordingNotifications(options: {
	addNotification: Notify
	addPopupNotification: Notify
	openSettings: () => void
	openVideos: () => void
}) {
	const queryClient = useQueryClient()
	let unlisten: (() => void) | undefined

	onMounted(async () => {
		unlisten = await onRecordingEvent((event) => {
			switch (event.type) {
				case 'started':
					options.addNotification({
						type: 'info',
						title: 'Grabando',
						text: `Grabando «${event.instance_name}». Pulsa la misma tecla para detener.`,
					})
					for (const warning of event.warnings) {
						options.addNotification({ type: 'warning', title: 'Grabación sin algunas pistas', text: warning })
					}
					break
				case 'saved':
					void queryClient.invalidateQueries({ queryKey: videoKeys.all })
					options.addPopupNotification({
						contentType: 'standard',
						title: 'Grabación guardada',
						text: `${event.video.file_name}${event.video.duration !== null ? ` · ${formatClock(event.video.duration)}` : ''}`,
						type: 'success',
						autoCloseMs: 12000,
						buttons: [{ label: 'Ver vídeos', color: 'brand', action: options.openVideos }],
					})
					break
				case 'discarded':
					options.addNotification({ type: 'info', title: 'Grabación descartada', text: event.reason })
					break
				case 'error':
					options.addNotification({ type: 'error', title: 'No se pudo grabar', text: event.message })
					break
				case 'needs-ffmpeg':
					options.addPopupNotification({
						contentType: 'standard',
						title: 'Falta el componente de grabación',
						text: 'Instálalo una vez en Ajustes → Grabación para poder grabar y recortar vídeos.',
						type: 'warning',
						autoCloseMs: null,
						buttons: [{ label: 'Abrir ajustes', color: 'brand', action: options.openSettings }],
					})
					break
				default:
					break
			}
		})
	})

	onBeforeUnmount(() => unlisten?.())
}
