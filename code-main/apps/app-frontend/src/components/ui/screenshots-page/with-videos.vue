<script setup lang="ts">
import { ImageIcon, VideoIcon } from '@modrinth/assets'
import { defineMessages, Tabs, type TabsTab, useVIntl } from '@modrinth/ui'
import { useStorage } from '@vueuse/core'

import VideosPage from '@/components/ui/videos/VideosPage.vue'

import ScreenshotsPage from './index.vue'

type Section = 'screenshots' | 'videos'

const props = withDefaults(defineProps<{ instanceId?: string; showHeading?: boolean }>(), {
	instanceId: undefined,
	showHeading: false,
})

const { formatMessage } = useVIntl()
const messages = defineMessages({
	heading: { id: 'app.screenshots.heading', defaultMessage: 'Screenshots' },
})

// La sección elegida se recuerda por separado en la pantalla global y en cada instancia.
const section = useStorage<Section>(`screenshots-section-${props.instanceId ? 'instance' : 'global'}`, 'screenshots')
const tabs: TabsTab[] = [
	{ value: 'screenshots', label: 'Capturas', icon: ImageIcon },
	{ value: 'videos', label: 'Vídeos', icon: VideoIcon },
]
</script>

<template>
	<div class="flex h-full w-full flex-col gap-3">
		<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
			<h1 v-if="showHeading" class="m-0 text-2xl font-bold text-contrast">
				{{ formatMessage(messages.heading) }}
			</h1>
			<Tabs
				:value="section"
				:tabs="tabs"
				color="gray"
				@update:value="(value) => (section = value as Section)"
			/>
		</div>
		<div class="min-h-0 flex-1">
			<ScreenshotsPage v-if="section === 'screenshots'" :instance-id="instanceId" />
			<VideosPage v-else :instance-id="instanceId" />
		</div>
	</div>
</template>
