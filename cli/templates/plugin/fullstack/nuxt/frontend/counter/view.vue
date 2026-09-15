<script setup lang="ts">
import { ref } from 'vue';
import { increment } from '../../shared/counter/service';
import { incrementOnServer } from '../../shared/counter/client';
const local = ref(0);
const remote = ref(0);
const message = ref('');
const busy = ref(false);
async function request() {
  busy.value = true;
  try {
    const result = await incrementOnServer(remote.value);
    remote.value = result.value;
    message.value = `租户：${result.tenant_id}`;
  } catch (error) { message.value = String(error); }
  finally { busy.value = false; }
}
</script>
<template>
  <section><h2>前端计数</h2><output aria-label="前端计数">{{ local }}</output><button @click="local = increment({ value: local })">前端 +1</button></section>
  <section><h2>后端计数</h2><output aria-label="后端计数">{{ remote }}</output><button :disabled="busy" @click="request">后端 +1</button><p role="status">{{ message }}</p></section>
</template>
