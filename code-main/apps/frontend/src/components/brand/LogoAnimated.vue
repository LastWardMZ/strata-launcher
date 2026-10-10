<template>
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" aria-label="Strata" class="strata-logo">
    <path fill="#70ddc5" d="M32 5 60 18 32 31 4 18Z" />
    <path fill="#3f8fa9" d="M4 29 32 42 60 29 60 38 32 51 4 38Z" />
    <path fill="#70ddc5" d="M4 43 32 56 60 43 60 51 32 64 4 51Z" />
  </svg>
</template>
<style scoped>
.strata-logo { width: 5rem; height: 5rem; animation: pulse 2s ease-in-out infinite; }
@keyframes pulse { 50% { opacity: .65; transform: scale(.94); } }
</style>