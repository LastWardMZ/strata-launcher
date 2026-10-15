<script setup lang="ts">
/** Pestaña «Mapas» del buscador: mapas de Minecraft de CurseForge. Mismo aspecto y comportamiento que las demás pestañas. */
import { CompassIcon, PlusIcon, SearchIcon } from '@modrinth/assets'
import { Admonition, Button, Combobox, defineMessages, Input, LoadingIndicator, NavTabs, Pagination, ProjectCard, ProjectCardList, useVIntl } from '@modrinth/ui'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'

import MapInstallModal from '@/components/ui/maps/MapInstallModal.vue'
import { messages } from '@/components/ui/maps/messages'
import { getMapCategories, type MapCategory, mapErrorMessage, type MapSort, type MapSummary, searchMaps } from '@/helpers/maps'
import { type BreadcrumbDefinition, injectBreadcrumbManager } from '@/providers/breadcrumbs'
import { get_game_versions } from '@/helpers/tags'

const { formatMessage } = useVIntl()
const route = useRoute()
const breadcrumbs = injectBreadcrumbManager()

// Mismos textos que las otras pestañas del buscador.
const tabMessages = defineMessages({
	modpacks: { id: 'app.browse.project-type.modpacks', defaultMessage: 'Modpacks' },
	mods: { id: 'app.browse.project-type.mods', defaultMessage: 'Mods' },
	resourcePacks: { id: 'app.browse.project-type.resource-packs', defaultMessage: 'Resource Packs' },
	dataPacks: { id: 'app.browse.project-type.data-packs', defaultMessage: 'Data Packs' },
	shaders: { id: 'app.browse.project-type.shaders', defaultMessage: 'Shaders' },
	servers: { id: 'app.browse.project-type.servers', defaultMessage: 'Servers' },
})

/** Instancia a la que se quiere añadir (`?i=`), si se llega desde una. */
const instanceId = computed(() => String(route.query.i ?? ''))
const tabs = computed(() => {
	const suffix = instanceId.value ? `?i=${encodeURIComponent(instanceId.value)}` : ''
	const withInstance = !!instanceId.value
	return [
		{ label: formatMessage(tabMessages.modpacks), href: `/browse/modpack${suffix}`, shown: !withInstance },
		{ label: formatMessage(tabMessages.mods), href: `/browse/mod${suffix}` },
		{ label: formatMessage(tabMessages.resourcePacks), href: `/browse/resourcepack${suffix}` },
		{ label: formatMessage(tabMessages.dataPacks), href: `/browse/datapack${suffix}` },
		{ label: formatMessage(tabMessages.shaders), href: `/browse/shader${suffix}` },
		{ label: formatMessage(messages.tab), href: `/browse/map${suffix}` },
		{ label: formatMessage(tabMessages.servers), href: `/browse/server${suffix}`, shown: !withInstance },
	]
})

const query = ref('')
const sort = ref<MapSort>('popular')
const version = ref('')
const category = ref<number | null>(null)
const page = ref(0)
const results = ref<MapSummary[]>([])
const total = ref(0)
const pageSize = ref(20)
const loading = ref(true)
const failure = ref('')
const categories = ref<MapCategory[]>([])
const versions = ref<string[]>([])
const installModal = ref<InstanceType<typeof MapInstallModal> | null>(null)

const sortOptions = computed(() => [
	{ value: 'popular' as MapSort, label: formatMessage(messages.sortPopular) },
	{ value: 'updated' as MapSort, label: formatMessage(messages.sortUpdated) },
	{ value: 'downloads' as MapSort, label: formatMessage(messages.sortDownloads) },
])
const versionOptions = computed(() => [{ value: '', label: formatMessage(messages.anyVersion) }, ...versions.value.map((v) => ({ value: v, label: v }))])
const categoryOptions = computed(() => [
	{ value: null as number | null, label: formatMessage(messages.anyCategory) },
	...categories.value.map((c) => ({ value: c.id as number | null, label: c.name })),
])
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize.value)))

let request = 0
async function load() {
	const mine = ++request
	loading.value = true
	failure.value = ''
	try {
		const result = await searchMaps({ q: query.value.trim(), version: version.value, category: category.value, sort: sort.value, page: page.value })
		if (mine !== request) return
		results.value = result.maps
		total.value = result.total
		pageSize.value = result.pageSize
	} catch (cause) {
		if (mine !== request) return
		results.value = []
		failure.value = mapErrorMessage(cause)
	} finally {
		if (mine === request) loading.value = false
	}
}

let timer: ReturnType<typeof setTimeout> | undefined
watch(query, () => {
	clearTimeout(timer)
	timer = setTimeout(() => {
		page.value = 0
		void load()
	}, 300)
})
watch([sort, version, category], () => {
	page.value = 0
	void load()
})

function setPage(next: number) {
	page.value = next - 1
	void load()
}

function linkOf(map: MapSummary) {
	return { path: `/maps/${map.id}`, query: instanceId.value ? { i: instanceId.value } : undefined }
}

const breadcrumb = {
	slot: 'browse',
	id: 'browse:map',
	label: () => formatMessage(messages.discover),
	to: () => route.fullPath,
	visual: { type: 'icon', component: CompassIcon },
} satisfies BreadcrumbDefinition

onMounted(async () => {
	breadcrumbs.reset(breadcrumb)
	void load()
	// Los filtros se rellenan cuando llegan; si fallan, la búsqueda sigue funcionando sin ellos.
	void getMapCategories().then((result) => (categories.value = result.categories)).catch(() => {})
	void get_game_versions()
		.then((list: { version: string; version_type: string }[]) => (versions.value = list.filter((v) => v.version_type === 'release').map((v) => v.version)))
		.catch(() => {})
})
onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
	<div class="flex flex-col gap-2 p-6">
		<NavTabs :links="tabs" replace />

		<Input
			v-model="query"
			:icon="SearchIcon"
			type="text"
			autocomplete="off"
			:placeholder="formatMessage(messages.searchPlaceholder)"
			clearable
			wrapper-class="w-full"
			size="large"
		/>

		<div class="flex flex-wrap items-center gap-2">
			<Combobox v-model="sort" :options="sortOptions" trigger-type="base" class="!w-[16rem] min-w-max max-w-full">
				<template #prefix>
					<span class="font-semibold text-primary">{{ formatMessage(messages.sortBy) }}</span>
				</template>
			</Combobox>
			<Combobox v-model="version" :options="versionOptions" trigger-type="base" searchable class="!w-[11rem] min-w-max max-w-full">
				<template #prefix>
					<span class="font-semibold text-primary">{{ formatMessage(messages.versionLabel) }}</span>
				</template>
			</Combobox>
			<Combobox v-model="category" :options="categoryOptions" trigger-type="base" class="!w-[16rem] min-w-max max-w-full">
				<template #prefix>
					<span class="font-semibold text-primary">{{ formatMessage(messages.categoryLabel) }}</span>
				</template>
			</Combobox>
			<Pagination :page="page + 1" :count="pageCount" class="ml-auto" @switch-page="setPage" />
		</div>

		<div class="search mt-1 [overflow-anchor:none]">
			<section v-if="loading" class="offline"><LoadingIndicator /></section>
			<Admonition v-else-if="failure" type="critical" :header="formatMessage(messages.loadError)" :body="failure">
				<template #actions>
					<Button @click="load">{{ formatMessage(messages.retry) }}</Button>
				</template>
			</Admonition>
			<section v-else-if="!results.length" class="offline text-center">
				<p>{{ formatMessage(messages.noResults) }}</p>
			</section>
			<ProjectCardList v-else layout="list">
				<ProjectCard
					v-for="map in results"
					:key="map.id"
					layout="list"
					:title="map.name"
					:icon-url="map.logo ?? undefined"
					:summary="map.summary"
					:author="map.authors.length ? { name: map.authors.slice(0, 2).join(', ') } : undefined"
					:tags="map.categories.map((c) => c.name)"
					:downloads="map.downloads"
					:date-updated="map.updated ?? undefined"
					:link="linkOf(map)"
					exclude-loaders
				>
					<template #actions>
						<Button type="outlined" color="brand" @click="installModal?.show(map, instanceId || undefined)">
							<PlusIcon /> {{ formatMessage(messages.install) }}
						</Button>
					</template>
				</ProjectCard>
			</ProjectCardList>
		</div>

		<p class="m-0 mt-2 text-sm text-secondary">{{ formatMessage(messages.attribution) }}</p>
		<MapInstallModal ref="installModal" />
	</div>
</template>
