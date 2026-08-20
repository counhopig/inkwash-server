<script setup lang="ts">
import { computed, inject, ref } from "vue";
import * as api from "./lib/api";
import { copyText } from "./lib/clipboard";
import { uiKey } from "./lib/ui";
import type { Device } from "./lib/types";

const props = defineProps<{
  devices: Device[];
  stats: Record<string, { alarms: number; todos: number; done: number }>;
}>();

const emit = defineEmits<{
  (e: "open-device", id: string): void;
  (e: "refresh"): void;
  (e: "open-account"): void;
}>();

const ui = inject(uiKey)!;

const newDeviceName = ref("");
const newDeviceToken = ref<string | null>(null);

const totals = computed(() => {
  let devices = 0;
  let alarms = 0;
  let todos = 0;
  let done = 0;
  for (const d of props.devices) {
    const s = props.stats[d.id];
    if (!s) continue;
    devices++;
    alarms += s.alarms;
    todos += s.todos;
    done += s.done;
  }
  return { devices, alarms, todos, done };
});

async function registerDevice() {
  const name = newDeviceName.value.trim();
  if (!name) {
    ui.toast("Enter a device name", "error");
    return;
  }
  const created = await ui.run("register-device", () =>
    api.registerDevice(ui.authToken(), name),
  );
  if (!created) return;
  newDeviceName.value = "";
  newDeviceToken.value = created.token ?? null;
  emit("refresh");
}

async function copyNewDeviceToken() {
  if (!newDeviceToken.value) return;
  const ok = await copyText(newDeviceToken.value);
  ui.toast(ok ? "Copied" : "Copy failed - select the token and copy manually", ok ? "info" : "error");
}

function openDevice(id: string) {
  emit("open-device", id);
}
</script>

<template>
  <div class="view">
    <div class="stat-row">
      <div class="stat">
        <span class="stat-num">{{ totals.devices }}</span>
        <span class="stat-label">Devices</span>
      </div>
      <div class="stat">
        <span class="stat-num">{{ totals.alarms }}</span>
        <span class="stat-label">Alarms</span>
      </div>
      <div class="stat">
        <span class="stat-num">{{ totals.todos }}</span>
        <span class="stat-label">Todos</span>
      </div>
      <div class="stat">
        <span class="stat-num">{{ totals.done }}</span>
        <span class="stat-label">Done</span>
      </div>
      <button class="quiet stat-refresh" type="button" @click="emit('refresh')">Refresh</button>
    </div>

    <section class="card" style="--i: 1">
      <div class="card-head">
        <h2><span class="index">01</span>Devices</h2>
        <span class="count">{{ devices.length }}</span>
      </div>
      <div v-if="devices.length === 0" class="empty">
        No devices yet - register one below to start managing content.
      </div>
      <div v-else class="device-list">
        <button
          v-for="d in devices"
          :key="d.id"
          class="device-card"
          type="button"
          @click="openDevice(d.id)"
        >
          <span class="device-name">{{ d.name }}</span>
          <span class="device-meta">
            {{ stats[d.id]?.alarms ?? 0 }} alarms · {{ stats[d.id]?.todos ?? 0 }} todos ·
            {{ stats[d.id]?.done ?? 0 }} done
          </span>
          <span class="device-open" aria-hidden="true">Open ▸</span>
        </button>
      </div>
    </section>

    <section class="card" style="--i: 2">
      <div class="card-head">
        <h2><span class="index">02</span>Register device</h2>
      </div>
      <form class="row" @submit.prevent="registerDevice">
        <input
          v-model="newDeviceName"
          class="grow"
          placeholder="Device name"
          :disabled="ui.isBusy('register-device')"
        />
        <button class="primary" type="submit" :disabled="ui.isBusy('register-device')">
          {{ ui.isBusy("register-device") ? "Registering…" : "Register" }}
        </button>
      </form>
      <div v-if="newDeviceToken" class="token-box">
        <div class="token-head">Device token <span class="meta">shown once</span></div>
        <code>{{ newDeviceToken }}</code>
        <div class="row" style="margin-top: 10px">
          <button class="quiet" type="button" @click="copyNewDeviceToken">Copy</button>
        </div>
      </div>
    </section>

    <p class="dashboard-hint">
      <span aria-hidden="true">◌</span> Select a device to manage the alarms and todos it receives
      on the next sync.
    </p>
  </div>
</template>
