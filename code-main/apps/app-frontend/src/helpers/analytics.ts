/**
 * Strata no recoge analítica ni telemetría: este módulo conserva los nombres y los tipos de los eventos para que el
 * resto del código siga compilando, pero no envía nada a ningún sitio (antes cargaba PostHog con la clave y el
 * servidor de Modrinth, siempre desactivado). `trackEvent` y las demás funciones no hacen nada.
 */

interface InstanceProperties {
	loader: string
	game_version: string
}

interface ProjectProperties extends InstanceProperties {
	id: string
	project_type: string
}

type AnalyticsEventMap = {
	Launched: { version: string; dev: boolean }
	PageView: { path: string; fromPath: string; failed: boolean }
	InstanceCreate: { source: string }
	InstanceCreateStart: { source: string }
	InstanceStart: InstanceProperties & { source: string }
	InstanceStop: Partial<InstanceProperties> & { source?: string }
	InstanceDuplicate: InstanceProperties
	InstanceRepair: InstanceProperties
	InstanceSetIcon: Record<string, never>
	InstanceRemoveIcon: Record<string, never>
	InstanceProjectUpdate: InstanceProperties & { id: string; name: string; project_type: string }
	InstanceProjectDisable: InstanceProperties & {
		id: string
		name: string
		project_type: string
		disabled: boolean
	}
	InstanceProjectRemove: InstanceProperties & { id: string; name: string; project_type: string }
	ProjectInstall: ProjectProperties & { version_id: string; title: string; source: string }
	ProjectInstallStart: { source: string }
	PackInstall: { id: string; version_id: string; title: string; source: string }
	PackInstallStart: Record<string, never>
	AccountLogIn: { source?: string }
	AccountLogOut: Record<string, never>
	JavaTest: { path: string; success: boolean }
	JavaManualSelect: { version: string }
	JavaAutoDetect: { path: string; version: string }
}

export type AnalyticsEvent = keyof AnalyticsEventMap

export const initAnalytics = () => {}
export const debugAnalytics = () => {}
export const optOutAnalytics = () => {}
export const optInAnalytics = () => {}

type OptionalArgs<T> = Record<string, never> extends T ? [properties?: T] : [properties: T]

export const trackEvent = <E extends AnalyticsEvent>(
	// eslint-disable-next-line @typescript-eslint/no-unused-vars
	_eventName: E,
	// eslint-disable-next-line @typescript-eslint/no-unused-vars
	..._args: OptionalArgs<AnalyticsEventMap[E]>
) => {}
