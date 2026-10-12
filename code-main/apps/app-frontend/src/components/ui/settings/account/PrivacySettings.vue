<script setup lang="ts">
import { Settings2Icon } from '@modrinth/assets'
import {
	Button,
	Chips,
	defineMessages,
	injectNotificationManager,
	injectPageContext,
	Toggle,
	useVIntl,
} from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { ref, watch } from 'vue'

import { open_ads_consent_preferences } from '@/helpers/ads.js'
import { optInAnalytics, optOutAnalytics } from '@/helpers/analytics'
import { get, set } from '@/helpers/settings.ts'

const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const { adConsentAvailable } = injectPageContext()
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
	adsConsentTitle: {
		id: 'app.ads-consent.title',
		defaultMessage: 'Your privacy and how ads support Modrinth',
	},
	adsConsentIntro: {
		id: 'app.settings.privacy.ads-consent.intro',
		defaultMessage:
			'Ads make Modrinth possible and fund creator payouts. Our partners may store or access cookies in the app to personalize ads and measure performance. You can opt out or manage your preferences below.',
	},
	adsConsentManage: {
		id: 'app.ads-consent.manage',
		defaultMessage: 'Manage preferences',
	},
	telemetryTitle: {
		id: 'app.settings.privacy.telemetry.title',
		defaultMessage: 'Telemetry',
	},
	telemetryDescription: {
		id: 'app.settings.privacy.telemetry.description',
		defaultMessage:
			"Strata doesn't collect or send usage data, so this setting has no effect. It's kept for compatibility.",
	},
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

async function manageAdsPreferences() {
	await open_ads_consent_preferences().catch(handleError)
}

watch(
	settings,
	async () => {
		if (settings.value.telemetry) {
			optInAnalytics()
		} else {
			optOutAnalytics()
		}

		await set(settings.value)
	},
	{ deep: true },
)
</script>

<template>
	<div v-if="adConsentAvailable">
		<h2 class="m-0 text-lg font-semibold text-contrast">
			{{ formatMessage(messages.adsConsentTitle) }}
		</h2>
		<div class="mt-2 flex flex-col gap-2.5 items-start">
			<Button @click="manageAdsPreferences">
				<Settings2Icon aria-hidden="true" />
				{{ formatMessage(messages.adsConsentManage) }}
			</Button>
			<div>
				{{ formatMessage(messages.adsConsentIntro) }}
			</div>
		</div>
	</div>

	<div class="mt-8 first:mt-0 flex items-center justify-between gap-4">
		<div>
			<h2 class="m-0 text-lg font-semibold text-contrast">
				{{ formatMessage(messages.telemetryTitle) }}
			</h2>
			<p class="m-0 mt-1">
				{{ formatMessage(messages.telemetryDescription) }}
			</p>
		</div>
		<Toggle id="opt-out-analytics" v-model="settings.telemetry" />
	</div>

	<div class="mt-4 flex items-center justify-between gap-4">
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
