<script setup lang="ts">
import { Card } from '@modrinth/ui'
import { computed } from 'vue'

/**
 * Resumen del tiempo de juego por instancia. Usa solo los contadores que el launcher ya guarda en este equipo; no se
 * envía nada a ningún sitio.
 */
type PlayedInstance = {
	id: string
	name: string
	recent_time_played?: number
	submitted_time_played?: number
}

const props = defineProps<{ instances: PlayedInstance[] }>()

const TOP = 5

const rows = computed(() =>
	props.instances
		.map((instance) => ({
			id: instance.id,
			name: instance.name,
			seconds: (instance.recent_time_played ?? 0) + (instance.submitted_time_played ?? 0),
		}))
		.filter((row) => row.seconds > 0)
		.sort((a, b) => b.seconds - a.seconds),
)
const total = computed(() => rows.value.reduce((sum, row) => sum + row.seconds, 0))
const top = computed(() => rows.value.slice(0, TOP))
const longest = computed(() => top.value[0]?.seconds ?? 1)

function duration(seconds: number) {
	const minutes = Math.round(seconds / 60)
	if (minutes < 60) return `${Math.max(minutes, 1)} min`
	const hours = Math.floor(minutes / 60)
	const rest = minutes % 60
	return rest ? `${hours} h ${rest} min` : `${hours} h`
}
</script>

<template>
	<Card v-if="total > 0" class="flex flex-col gap-3">
		<div class="flex flex-wrap items-baseline justify-between gap-2">
			<h2 class="m-0 text-lg font-bold text-contrast">Tu tiempo de juego</h2>
			<span class="text-sm text-secondary">
				{{ duration(total) }} en {{ rows.length }} {{ rows.length === 1 ? 'instancia' : 'instancias' }}
			</span>
		</div>
		<ol class="m-0 flex list-none flex-col gap-2 p-0">
			<li v-for="row in top" :key="row.id" class="flex flex-col gap-1">
				<div class="flex items-center justify-between gap-3 text-sm">
					<router-link
						:to="`/instance/${encodeURIComponent(row.id)}`"
						class="min-w-0 truncate font-semibold text-contrast no-underline hover:underline focus-visible:underline"
					>
						{{ row.name }}
					</router-link>
					<span class="shrink-0 tabular-nums text-secondary">{{ duration(row.seconds) }}</span>
				</div>
				<div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-5" aria-hidden="true">
					<div
						class="h-full rounded-full bg-brand"
						:style="{ width: `${Math.max(3, (row.seconds / longest) * 100)}%` }"
					/>
				</div>
			</li>
		</ol>
		<p v-if="rows.length > TOP" class="m-0 text-xs text-secondary">
			Y {{ rows.length - TOP }} más con menos tiempo.
		</p>
	</Card>
</template>
