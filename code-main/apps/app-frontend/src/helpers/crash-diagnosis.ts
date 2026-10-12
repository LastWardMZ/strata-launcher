/**
 * Diagnóstico local de cierres inesperados de Minecraft. Lee el registro de la instancia y busca los errores más
 * comunes con reglas fijas: no sale nada del equipo (antes el registro entero se enviaba a un servicio externo).
 *
 * Devuelve la misma forma que la consola compartida espera (`analysis.problems[].message / solutions[].message`).
 */
export type Problem = { message: string; solutions: { message: string }[] }
export type Diagnosis = { analysis: { problems: Problem[] } }

type Rule = {
	/** Texto del registro que dispara la regla; el grupo 1, si existe, se pasa a `build`. */
	pattern: RegExp
	build: (match: RegExpExecArray) => Problem
}

const solutions = (...messages: string[]) => messages.map((message) => ({ message }))

/** Versión de clase de Java → versión de Java (52 → 8, 61 → 17, 65 → 21). */
const javaForClassVersion = (classVersion: number) => classVersion - 44

const RULES: Rule[] = [
	{
		pattern: /java\.lang\.OutOfMemoryError|GC overhead limit exceeded/,
		build: () => ({
			message: 'Minecraft se quedó sin memoria',
			solutions: solutions(
				'Sube la memoria asignada en los ajustes de la instancia (Java y memoria). Con modpacks grandes suelen hacer falta 6–8 GB.',
				'Quita mods, shaders o texturas pesadas, o reduce la distancia de renderizado.',
				'Cierra otros programas: si el equipo se queda sin memoria libre, Minecraft se detiene aunque tenga asignada más.',
			),
		}),
	},
	{
		pattern: /class file version (\d+)\.\d+/,
		build: (match) => {
			const needed = javaForClassVersion(Number(match[1]))
			return {
				message: `Esta versión de Java es demasiado antigua: hace falta Java ${needed} o posterior`,
				solutions: solutions(
					`En los ajustes de la instancia, instala o elige Java ${needed}.`,
					'Si usas una ruta de Java propia, comprueba que apunta a una versión reciente.',
				),
			}
		},
	},
	{
		pattern: /requires? Java (\d+)/i,
		build: (match) => ({
			message: `Esta combinación de Minecraft y mods necesita Java ${match[1]}`,
			solutions: solutions(`En los ajustes de la instancia, instala o elige Java ${match[1]}.`),
		}),
	},
	{
		pattern: /[Dd]uplicate mods? found|found a duplicate mod|Found duplicate mods/,
		build: () => ({
			message: 'Hay mods duplicados',
			solutions: solutions(
				'Abre la carpeta de mods y deja solo una copia de cada uno (cuenta también versiones distintas del mismo mod).',
			),
		}),
	},
	{
		pattern: /Missing or unsupported mandatory dependencies|Could not find required mod|which is missing!?/,
		build: () => ({
			message: 'A un mod le falta otro mod del que depende',
			solutions: solutions(
				'Lee en el registro el nombre del mod que falta (aparece junto a «requires» o «missing») e instálalo en la misma versión de Minecraft y del cargador.',
				'Si acabas de actualizar, puede que el mod necesario aún no exista para esta versión: actualiza o quita el mod que lo pide.',
			),
		}),
	},
	{
		pattern: /Mixin apply failed|MixinApplyError|InvalidInjectionException|Mixin transformation of .* failed/,
		build: () => ({
			message: 'Un mod no es compatible con otro mod o con esta versión de Minecraft',
			solutions: solutions(
				'Busca en el registro el nombre del archivo «….mixins.json» o del mod que aparece en el error: ese es el sospechoso.',
				'Actualiza ese mod (y el cargador) o quítalo para comprobar que el juego arranca sin él.',
			),
		}),
	},
	{
		pattern: /NoClassDefFoundError|NoSuchMethodError|NoSuchFieldError|ClassNotFoundException: (?:net|com|org)\./,
		build: () => ({
			message: 'Un mod no coincide con la versión del juego o de otro mod',
			solutions: solutions(
				'Comprueba que todos los mods son de la misma versión de Minecraft y del mismo cargador (Fabric, Forge, NeoForge o Quilt).',
				'Actualiza los mods que se hayan quedado atrás; si el error empezó tras instalar uno, quítalo.',
			),
		}),
	},
	{
		pattern: /EXCEPTION_ACCESS_VIOLATION|GLFW error 65543|GLFW error 65542|WGL: The driver does not appear to support OpenGL|Pixel format not accelerated/,
		build: () => ({
			message: 'Problema con la tarjeta gráfica o sus controladores',
			solutions: solutions(
				'Actualiza los controladores de la tarjeta gráfica desde la web de NVIDIA, AMD o Intel.',
				'Cierra programas que se enganchan al juego (overlays de grabación, monitores de FPS) y vuelve a probar.',
				'Si usas shaders o mods de rendimiento, desactívalos para comprobar si son la causa.',
			),
		}),
	},
	{
		pattern: /Address already in use|java\.net\.BindException|Failed to bind to port/,
		build: () => ({
			message: 'El puerto del servidor ya está en uso',
			solutions: solutions(
				'Cierra el otro servidor o programa que use ese puerto, o cambia el puerto del servidor en sus ajustes.',
				'Si acabas de cerrar el servidor, espera unos segundos: Windows tarda un poco en liberar el puerto.',
			),
		}),
	},
]

/** Analiza el texto del registro; `null` si no reconoce ningún problema. */
export function diagnoseLog(text: string): Diagnosis | null {
	if (!text) return null
	const problems: Problem[] = []
	for (const rule of RULES) {
		const match = rule.pattern.exec(text)
		if (match) problems.push(rule.build(match))
	}
	return problems.length ? { analysis: { problems } } : null
}
