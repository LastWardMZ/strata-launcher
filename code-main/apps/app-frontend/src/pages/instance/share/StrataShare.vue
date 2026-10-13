<script setup lang="ts">
import { UsersIcon } from '@modrinth/assets'
import { Button, EmptyState } from '@modrinth/ui'
import { useRouter } from 'vue-router'

/**
 * Compartir una instancia con amigos. En Strata se hace por P2P desde «Chat y partidas»: eliges al amigo, pulsas
 * «Compartir instancia…» y le llega una receta (versión, cargador y contenido) para instalarla igual.
 */
const router = useRouter()
</script>

<template>
	<div class="flex w-full justify-center p-6">
		<EmptyState
			type="empty"
			heading="Comparte esta instancia con un amigo"
			description="En Chat y partidas eliges a tu amigo y usas «Compartir instancia…»: recibirá la misma versión, el mismo cargador y el mismo contenido para instalarla con un clic. Va directo entre vosotros, sin servidores de por medio."
		>
			<template #actions>
				<Button type="colored" color="brand" @click="router.push('/friends')">
					<UsersIcon /> Abrir Chat y partidas
				</Button>
			</template>
		</EmptyState>
	</div>
</template>
