<script setup lang="ts">
import { Settings2Icon } from '@modrinth/assets'
import {
	Button,
	Chips,
	defineMessages,
	injectNotificationManager,
	Toggle,
	useVIntl,
} from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { ref, watch } from 'vue'

import { get, set } from '@/helpers/settings.ts'

const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const settings = ref(await get())

// Nivel de detalle de Discord (completo o básico); es un ajuste local, aparte de los demás.
const discordDetail = ref<'full' | 'basic'>('full')
invoke<string>('discord_detail_get')
	.then((level) => (discordDetail.value = level === 'basic' ? 'basic' : 'full'))
	.catch(() => {})
async function setDiscordDetail(level: string | null | undefined) {
	if (level !== 'full' && level !== 'basic') return
	try {
		await invoke('discord_detail_set', { level })
		discordDetail.value = level
	} catch (error) {
		handleError(error as Error)
	}
}

const messages = defineMessages({
	discordRichPresenceTitle: {
		id: 'app.settings.privacy.discord-rich-presence.title',
		defaultMessage: 'Discord Rich Presence',
	},
	discordRichPresenceDescription: {
		id: 'app.settings.privacy.discord-rich-presence.description',
		defaultMessage:
			'Show Strata as your current activity on Discord. This does not affect Rich Presence added to instances by mods. Requires an app restart.',
	},
})

watch(
	settings,
	async () => {
		await set(settings.value)
	},
	{ deep: true },
)
</script>

<template>
	<div class="flex items-center justify-between gap-4">
		<div>
			<h2 class="m-0 text-lg font-semibold text-contrast">
				{{ formatMessage(messages.discordRichPresenceTitle) }}
			</h2>
			<p class="m-0 mt-1">
				{{ formatMessage(messages.discordRichPresenceDescription) }}
			</p>
		</div>
		<Toggle id="disable-discord-rpc" v-model="settings.discord_rpc" />
	</div>

	<div v-if="settings.discord_rpc" class="mt-3 flex flex-col gap-2">
		<h3 class="m-0 text-base font-semibold text-contrast">Detalle en Discord</h3>
		<Chips
			:model-value="discordDetail"
			:items="['full', 'basic']"
			:format-label="(value: string) => (value === 'full' ? 'Completo' : 'Básico')"
			:capitalize="false"
			aria-label="Detalle en Discord"
			@update:model-value="setDiscordDetail"
		/>
		<p class="m-0 text-secondary">
			{{
				discordDetail === 'full'
					? 'Muestra el nombre de la instancia, la versión de Minecraft y el cargador.'
					: 'Solo muestra «Jugando a Minecraft»: sin nombre de instancia ni versión.'
			}}
		</p>
	</div>
</template>
