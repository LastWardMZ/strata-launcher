<script setup lang="ts">
import { computed, ref, watch } from 'vue'

export type PresenceStatus = 'online' | 'offline' | 'playing' | 'busy'

const props = withDefaults(
	defineProps<{
		name: string
		/** Imagen del usuario; sin ella se dibuja su inicial sobre un color propio de cada nombre. */
		src?: string | null
		size?: number
		/** Si se indica, se dibuja el punto de presencia y se anuncia en el texto accesible. */
		status?: PresenceStatus | null
	}>(),
	{ src: null, size: 32, status: null },
)

const failed = ref(false)
watch(
	() => props.src,
	() => (failed.value = false),
)

const initial = computed(() => (props.name.trim()[0] ?? '?').toUpperCase())
// Mismo nombre, mismo color; la luminosidad baja garantiza contraste con el texto blanco.
const background = computed(() => {
	let hash = 0
	for (const char of props.name.toLowerCase()) hash = (hash * 31 + char.charCodeAt(0)) % 360
	return `hsl(${hash} 42% 32%)`
})
const statusText = computed(() =>
	props.status === 'playing'
		? 'jugando'
		: props.status === 'busy'
			? 'no molestar'
			: props.status === 'online'
				? 'en línea'
				: 'desconectado',
)
const dot = computed(() => Math.max(8, Math.round(props.size * 0.3)))
</script>

<template>
	<span
		class="relative inline-flex shrink-0"
		role="img"
		:aria-label="status ? `${name}, ${statusText}` : name"
		:style="{ width: `${size}px`, height: `${size}px` }"
	>
		<img
			v-if="src && !failed"
			:src="src"
			alt=""
			draggable="false"
			class="size-full rounded-full object-cover"
			@error="failed = true"
		/>
		<span
			v-else
			class="flex size-full items-center justify-center rounded-full font-bold text-white"
			:style="{ background, fontSize: `${Math.round(size * 0.42)}px` }"
			aria-hidden="true"
			>{{ initial }}</span
		>
		<span
			v-if="status"
			class="absolute -bottom-0.5 -right-0.5 rounded-full border-2 border-solid border-surface-2"
			:class="
				status === 'offline'
					? 'bg-surface-5'
					: status === 'playing'
						? 'bg-brand'
						: status === 'busy'
							? 'bg-red'
							: 'bg-green'
			"
			:style="{ width: `${dot}px`, height: `${dot}px` }"
			aria-hidden="true"
		/>
	</span>
</template>
