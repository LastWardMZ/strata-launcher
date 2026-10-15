<script setup lang="ts">
/** Ficha de un mapa de CurseForge: galería, descripción, autores, versiones y enlace a CurseForge. */
import { CompassIcon, ExternalIcon, PlusIcon } from '@modrinth/assets'
import { Admonition, Avatar, Button, LoadingIndicator, useVIntl } from '@modrinth/ui'
import { renderExternalHtml } from '@modrinth/utils'
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import MapInstallModal from '@/components/ui/maps/MapInstallModal.vue'
import { messages } from '@/components/ui/maps/messages'
import { getMap, type MapDetail, mapErrorMessage } from '@/helpers/maps'
import { type BreadcrumbDefinition, injectBreadcrumbManager } from '@/providers/breadcrumbs'

const { formatMessage } = useVIntl()
const route = useRoute()
const router = useRouter()
const breadcrumbs = injectBreadcrumbManager()

const map = ref<MapDetail | null>(null)
const loading = ref(true)
const failure = ref('')
const shot = ref(0)
const installModal = ref<InstanceType<typeof MapInstallModal> | null>(null)

/** Instancia desde la que se llegó (`?i=`): se preselecciona al instalar. */
const instanceId = computed(() => String(route.query.i ?? ''))
const html = computed(() => (map.value ? renderExternalHtml(map.value.html) : ''))
const authors = computed(() => map.value?.authors.join(', ') ?? '')
const shots = computed(() => map.value?.screenshots ?? [])
const numberFormat = new Intl.NumberFormat(undefined, { notation: 'compact' })
const day = (value: string | null) => (value ? new Date(value).toLocaleDateString() : '')

function openCurseForge() {
	if (map.value?.url) void openUrl(map.value.url)
}

function back() {
	if (window.history.length > 1) router.back()
	else void router.push({ path: '/browse/map', query: instanceId.value ? { i: instanceId.value } : undefined })
}

onMounted(async () => {
	const id = Number(route.params.id)
	const browse = breadcrumbs.reset({
		slot: 'browse',
		id: 'browse:map',
		label: () => formatMessage(messages.discover),
		to: '/browse/map',
		visual: { type: 'icon', component: CompassIcon },
	} satisfies BreadcrumbDefinition)
	try {
		if (!Number.isInteger(id) || id <= 0) throw new Error(formatMessage(messages.notFound))
		map.value = await getMap(id)
		breadcrumbs.push({ slot: 'project', id: `map:${id}`, label: () => map.value?.name ?? '', to: route.fullPath, visual: { type: 'icon', component: CompassIcon } } satisfies BreadcrumbDefinition, { parent: browse })
	} catch (cause) {
		failure.value = mapErrorMessage(cause) || formatMessage(messages.notFound)
	} finally {
		loading.value = false
	}
})
</script>

<template>
	<div class="flex flex-col gap-4 p-6">
		<div><Button @click="back">← {{ formatMessage(messages.backToBrowse) }}</Button></div>

		<section v-if="loading" class="flex justify-center p-8"><LoadingIndicator /></section>
		<Admonition v-else-if="failure" type="critical" :header="formatMessage(messages.notFound)" :body="failure" />

		<template v-else-if="map">
			<header class="flex flex-wrap items-start gap-4 rounded-2xl border border-solid border-surface-4 bg-surface-3 p-4">
				<Avatar :src="map.logoFull ?? map.logo ?? undefined" size="96px" />
				<div class="flex min-w-0 flex-1 flex-col gap-1">
					<h1 class="m-0 text-2xl font-bold text-contrast">{{ map.name }}</h1>
					<p v-if="authors" class="m-0 text-secondary">{{ formatMessage(messages.byAuthors, { authors }) }}</p>
					<p class="m-0">{{ map.summary }}</p>
					<div v-if="map.categories.length" class="mt-1 flex flex-wrap gap-1.5">
						<span v-for="category in map.categories" :key="category.id" class="rounded-full bg-surface-4 px-2.5 py-0.5 text-xs text-secondary">{{ category.name }}</span>
					</div>
				</div>
				<div class="flex flex-col gap-2">
					<Button v-if="map.distributable" type="colored" color="brand" @click="installModal?.show(map, instanceId || undefined)">
						<PlusIcon /> {{ formatMessage(messages.install) }}
					</Button>
					<Button v-else type="colored" color="brand" @click="openCurseForge">
						<ExternalIcon /> {{ formatMessage(messages.downloadOnCurseForge) }}
					</Button>
					<Button v-if="map.url && map.distributable" @click="openCurseForge">
						<ExternalIcon /> {{ formatMessage(messages.viewOnCurseForge) }}
					</Button>
				</div>
			</header>

			<Admonition v-if="!map.distributable" type="info" :body="formatMessage(messages.noDirectDownload)" />

			<div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_18rem]">
				<div class="flex min-w-0 flex-col gap-4">
					<section v-if="shots.length" class="flex flex-col gap-2">
						<h2 class="m-0 text-lg font-semibold text-contrast">{{ formatMessage(messages.galleryTitle) }}</h2>
						<img :src="shots[shot].url" :alt="shots[shot].title" class="max-h-[28rem] w-full rounded-2xl border border-solid border-surface-4 bg-surface-2 object-contain" />
						<div v-if="shots.length > 1" class="flex gap-2 overflow-x-auto pb-1">
							<button
								v-for="(item, index) in shots"
								:key="item.url"
								type="button"
								class="shrink-0 cursor-pointer rounded-lg border-2 border-solid bg-transparent p-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
								:class="index === shot ? 'border-brand' : 'border-transparent'"
								:aria-label="item.title || `${index + 1}`"
								@click="shot = index"
							>
								<img :src="item.thumbnail ?? item.url" alt="" class="h-16 w-28 rounded-md object-cover" />
							</button>
						</div>
					</section>
					<section class="flex flex-col gap-2">
						<h2 class="m-0 text-lg font-semibold text-contrast">{{ formatMessage(messages.descriptionTitle) }}</h2>
						<div class="markdown-body rounded-2xl border border-solid border-surface-4 bg-surface-3 p-4" v-html="html" />
					</section>
				</div>

				<aside class="flex flex-col gap-3">
					<section class="flex flex-col gap-1.5 rounded-2xl border border-solid border-surface-4 bg-surface-3 p-4">
						<h2 class="m-0 text-base font-semibold text-contrast">{{ formatMessage(messages.detailsTitle) }}</h2>
						<p class="m-0 text-sm">{{ formatMessage(messages.downloadsLabel, { count: numberFormat.format(map.downloads) }) }}</p>
						<p v-if="map.updated" class="m-0 text-sm">{{ formatMessage(messages.updatedLabel, { date: day(map.updated) }) }}</p>
					</section>
					<section v-if="map.versions.length" class="flex flex-col gap-1.5 rounded-2xl border border-solid border-surface-4 bg-surface-3 p-4">
						<h2 class="m-0 text-base font-semibold text-contrast">{{ formatMessage(messages.versionsTitle) }}</h2>
						<div class="flex flex-wrap gap-1.5">
							<span v-for="version in map.versions" :key="version" class="rounded-full bg-surface-4 px-2.5 py-0.5 text-xs text-secondary">{{ version }}</span>
						</div>
					</section>
					<p class="m-0 text-xs text-secondary">{{ formatMessage(messages.attribution) }}</p>
				</aside>
			</div>
		</template>

		<MapInstallModal ref="installModal" />
	</div>
</template>
