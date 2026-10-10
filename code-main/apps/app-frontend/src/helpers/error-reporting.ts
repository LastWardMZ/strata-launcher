import type { App } from 'vue'
import type { Router } from 'vue-router'

/**
 * Strata no envía informes de errores a terceros. La versión original de
 * Modrinth cargaba Sentry y mandaba las excepciones a su proyecto; aquí los
 * errores solo se muestran en la consola local.
 */
export function setupErrorReporting(app: App, _router: Router): void {
	const previousHandler = app.config.errorHandler
	app.config.errorHandler = (error, instance, info) => {
		if (previousHandler) previousHandler(error, instance, info)
		else console.error(error)
	}
}
