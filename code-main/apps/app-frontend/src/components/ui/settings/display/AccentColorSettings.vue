<script setup lang="ts">
import { CheckIcon, UndoIcon } from '@modrinth/assets'
import { Button, ColorPicker } from '@modrinth/ui'
import { computed } from 'vue'

import {
	ACCENT_PRESETS,
	accentColor,
	DEFAULT_ACCENT,
	resetAccentColor,
	setAccentColor,
} from '@/helpers/accent-color'

const custom = computed({
	get: () => accentColor.value,
	set: (value: string) => void setAccentColor(value),
})
const isPreset = computed(() => ACCENT_PRESETS.some((preset) => preset.color === accentColor.value))
</script>

<template>
	<section class="mt-6 flex flex-col gap-3">
		<div>
			<h2 class="m-0 text-lg font-extrabold text-contrast">Color de acento</h2>
			<p class="m-0 mt-1 text-secondary">
				Elige el color de los botones, enlaces y resaltados del launcher. Se aplica al momento y se
				adapta solo a los temas claro y oscuro.
			</p>
		</div>

		<div class="flex flex-wrap items-center gap-2" role="radiogroup" aria-label="Color de acento">
			<button
				v-for="preset in ACCENT_PRESETS"
				:key="preset.id"
				v-tooltip="preset.name"
				type="button"
				role="radio"
				:aria-checked="accentColor === preset.color"
				:aria-label="preset.name"
				class="flex size-9 cursor-pointer items-center justify-center rounded-xl border-0 text-black transition-transform hover:scale-105 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-brand-shadow"
				:class="accentColor === preset.color ? 'ring-2 ring-contrast' : 'ring-1 ring-surface-5'"
				:style="{ backgroundColor: preset.color }"
				@click="setAccentColor(preset.color)"
			>
				<CheckIcon v-if="accentColor === preset.color" class="size-5" />
			</button>

			<span class="mx-1 h-6 w-px bg-surface-5" aria-hidden="true" />

			<div class="flex items-center gap-2">
				<ColorPicker v-model="custom" label="Color personalizado" />
				<span class="text-sm" :class="isPreset ? 'text-secondary' : 'font-semibold text-contrast'">
					{{ isPreset ? 'Personalizado' : `Personalizado · ${accentColor.toUpperCase()}` }}
				</span>
			</div>

			<Button
				type="outlined"
				size="sm"
				class="ml-auto"
				:disabled="accentColor === DEFAULT_ACCENT"
				@click="resetAccentColor"
			>
				<UndoIcon /> Restablecer
			</Button>
		</div>
	</section>
</template>
