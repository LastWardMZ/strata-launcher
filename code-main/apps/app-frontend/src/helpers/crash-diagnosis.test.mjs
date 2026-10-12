// Prueba del diagnóstico local de cierres. Se ejecuta con: node --test src/helpers/crash-diagnosis.test.mjs
import assert from 'node:assert/strict'
import test from 'node:test'

import { diagnoseLog } from './crash-diagnosis.ts'

const messages = (text) => diagnoseLog(text)?.analysis.problems.map((p) => p.message) ?? []

test('un registro normal no genera avisos', () => {
	assert.equal(diagnoseLog(''), null)
	assert.equal(diagnoseLog('[12:00:00] [main/INFO]: Setting user: Steve\n[12:00:03] [Render thread/INFO]: Stopping!'), null)
})

test('memoria agotada', () => {
	const found = diagnoseLog('Exception in thread "main" java.lang.OutOfMemoryError: Java heap space')
	assert.deepEqual(found.analysis.problems.map((p) => p.message), ['Minecraft se quedó sin memoria'])
	assert.ok(found.analysis.problems[0].solutions.length >= 2)
})

test('Java antiguo: calcula la versión que hace falta a partir de la versión de clase', () => {
	const log = 'java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0), this version of the Java Runtime only recognizes class file versions up to 61.0'
	assert.deepEqual(messages(log), ['Esta versión de Java es demasiado antigua: hace falta Java 21 o posterior'])
	assert.match(diagnoseLog('class file version 61.0').analysis.problems[0].message, /Java 17/)
	assert.match(diagnoseLog('class file version 52.0').analysis.problems[0].message, /Java 8/)
})

test('mods duplicados, dependencias que faltan y mixins', () => {
	assert.deepEqual(messages('Found duplicate mods:\n\tsodium'), ['Hay mods duplicados'])
	assert.deepEqual(
		messages("Mod 'Iris' (iris) 1.7 requires version 1.6.0 of mod sodium, which is missing!"),
		['A un mod le falta otro mod del que depende'],
	)
	assert.deepEqual(
		messages('org.spongepowered.asm.mixin.transformer.throwables.MixinApplyError: Mixin [foo.mixins.json:Bar] apply failed'),
		['Un mod no es compatible con otro mod o con esta versión de Minecraft'],
	)
})

test('versiones que no coinciden, gráficos y puerto en uso', () => {
	assert.deepEqual(messages('java.lang.NoSuchMethodError: net.minecraft.class_123.method_456()V'), ['Un mod no coincide con la versión del juego o de otro mod'])
	assert.deepEqual(messages('# EXCEPTION_ACCESS_VIOLATION (0xc0000005) at pc=0x00007ff'), ['Problema con la tarjeta gráfica o sus controladores'])
	assert.deepEqual(messages('java.net.BindException: Address already in use: bind'), ['El puerto del servidor ya está en uso'])
})

test('varios problemas a la vez se devuelven todos, en un orden estable', () => {
	const log = 'java.lang.OutOfMemoryError: Java heap space\nFound duplicate mods'
	assert.deepEqual(messages(log), ['Minecraft se quedó sin memoria', 'Hay mods duplicados'])
})
