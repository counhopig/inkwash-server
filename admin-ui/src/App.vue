<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import * as api from "./lib/api";
import { copyText } from "./lib/clipboard";
import { loadAdminToken, saveAdminToken } from "./lib/storage";
import type { Alarm, Device, Repeat, Todo, TodoDue } from "./lib/types";

const token = ref(loadAdminToken());
const connected = ref(false);

const devices = ref<Device[]>([]);
const selectedDeviceId = ref<string | null>(null);
const alarms = ref<Alarm[]>([]);
const todos = ref<Todo[]>([]);

const newDeviceName = ref("");
const newDeviceToken = ref<string | null>(null);

const alarmHour = ref(7);
const alarmMinute = ref(0);
const alarmLabel = ref("");
const alarmRepeatKind = ref<"Daily" | "Weekly" | "Monthly" | "Once">("Daily");
const alarmRepeatDays = ref("");
const alarmOnceDate = ref("");

const todoText = ref("");
const todoImportance = ref<"low" | "medium" | "high">("medium");
const todoDue = ref("");
const todoRepeatKind = ref<"" | "Daily" | "Weekly" | "Monthly">("");
const todoRepeatDays = ref("");

const toastText = ref("");
const toastVisible = ref(false);
let toastTimer: ReturnType<typeof setTimeout> | undefined;

function toast(text: string) {
  toastText.value = text;
  toastVisible.value = true;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toastVisible.value = false), 3000);
}

const selectedDevice = computed(
  () => devices.value.find((d) => d.id === selectedDeviceId.value) ?? null,
);

async function connect() {
  saveAdminToken(token.value.trim());
  token.value = token.value.trim();
  await loadDevices();
}

async function loadDevices() {
  const previous = selectedDeviceId.value;
  const r = await api.listDevices(token.value);
  if (!r.ok) {
    connected.value = false;
    toast(r.error);
    return;
  }
  devices.value = r.value;
  connected.value = true;
  if (previous != null && r.value.some((d) => d.id === previous)) {
    selectedDeviceId.value = previous;
  } else {
    selectedDeviceId.value = null;
  }
  await loadContent();
}

async function registerDevice() {
  const name = newDeviceName.value.trim();
  if (!name) {
    toast("Enter a device name");
    return;
  }
  const r = await api.registerDevice(token.value, name);
  if (!r.ok) {
    toast(r.error);
    return;
  }
  newDeviceName.value = "";
  newDeviceToken.value = r.value.token ?? null;
  await loadDevices();
  selectedDeviceId.value = r.value.id;
  await loadContent();
}

async function copyNewDeviceToken() {
  if (!newDeviceToken.value) return;
  const ok = await copyText(newDeviceToken.value);
  toast(ok ? "Copied" : "Copy failed - select the token and copy manually");
}

async function deleteDevice(id: string) {
  if (!confirm("Delete this device and all its content?")) return;
  const r = await api.deleteDevice(token.value, id);
  if (!r.ok) {
    toast(r.error);
    return;
  }
  newDeviceToken.value = null;
  if (selectedDeviceId.value === id) selectedDeviceId.value = null;
  await loadDevices();
}

function pickDevice(id: string) {
  newDeviceToken.value = null;
  selectedDeviceId.value = id;
  loadContent();
}

async function loadContent() {
  const id = selectedDeviceId.value;
  if (id == null) {
    alarms.value = [];
    todos.value = [];
    return;
  }
  const [a, t] = await Promise.all([api.listAlarms(token.value, id), api.listTodos(token.value, id)]);
  if (a.ok) alarms.value = a.value;
  else toast(a.error);
  if (t.ok) todos.value = t.value;
  else toast(t.error);
}

async function addAlarm() {
  const id = selectedDeviceId.value;
  if (id == null) {
    toast("Select a device");
    return;
  }
  const r = await api.createAlarm(token.value, id, {
    hour: alarmHour.value,
    minute: alarmMinute.value,
    repeat: buildRepeat(alarmRepeatKind.value, alarmRepeatDays.value, alarmOnceDate.value),
    enabled: true,
    label: alarmLabel.value.trim(),
  });
  if (!r.ok) {
    toast(r.error);
    return;
  }
  alarmLabel.value = "";
  await loadContent();
  toast("Alarm added");
}

function buildRepeat(kind: string, daysRaw: string, onceDate: string): Repeat {
  if (kind === "Weekly" || kind === "Monthly") {
    const days = daysRaw
      .split(",")
      .map((s) => Number(s.trim()))
      .filter((n) => Number.isFinite(n));
    return kind === "Weekly" ? { Weekly: { days } } : { Monthly: { days } };
  }
  if (kind === "Once") {
    const m = onceDate.match(/^(\d{4})-(\d{1,2})-(\d{1,2})$/);
    if (m) {
      return { Once: { year: Number(m[1]), month: Number(m[2]), day: Number(m[3]) } };
    }
  }
  return "Daily";
}

async function deleteAlarm(alarmId: number) {
  const id = selectedDeviceId.value;
  if (id == null) return;
  const r = await api.deleteAlarm(token.value, id, alarmId);
  if (!r.ok) toast(r.error);
  else await loadContent();
}

async function clearAlarms() {
  const id = selectedDeviceId.value;
  if (id == null || !confirm("Clear all alarms?")) return;
  const r = await api.clearAlarms(token.value, id);
  if (!r.ok) toast(r.error);
  else {
    await loadContent();
    toast("All alarms cleared");
  }
}

function parseDueDate(raw: string): TodoDue | null {
  const m = raw.trim().match(/^(\d{4})-(\d{1,2})-(\d{1,2})$/);
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  return { year, month, day };
}

async function addTodo() {
  const id = selectedDeviceId.value;
  const text = todoText.value.trim();
  if (id == null || !text) {
    toast("Select a device and enter text");
    return;
  }
  const due = todoDue.value.trim();
  const dueDate = due ? parseDueDate(due) : null;
  if (due && !dueDate) {
    toast("Due date must be YYYY-MM-DD");
    return;
  }
  const repeat = todoRepeatKind.value
    ? buildRepeat(todoRepeatKind.value, todoRepeatDays.value, "")
    : null;
  const r = await api.createTodo(token.value, id, {
    text,
    done: false,
    importance: todoImportance.value,
    due_date: dueDate,
    repeat,
  });
  if (!r.ok) {
    toast(r.error);
    return;
  }
  todoText.value = "";
  todoDue.value = "";
  todoRepeatKind.value = "";
  todoRepeatDays.value = "";
  await loadContent();
  toast("Todo added");
}

function todoBadge(importance: "low" | "medium" | "high"): string {
  return importance === "high" ? "H" : importance === "medium" ? "M" : "L";
}

function formatDue(t: Todo): string {
  if (!t.due_date) return "";
  const d = t.due_date;
  return `${pad(d.year)}-${pad(d.month)}-${pad(d.day)}`;
}

function repeatLabel(r: Repeat | null): string {
  if (!r) return "Once";
  if (r === "Daily") return "Daily";
  if ("Weekly" in r) return `Weekly [${r.Weekly.days.join(",")}]`;
  if ("Monthly" in r) return `Monthly [${r.Monthly.days.join(",")}]`;
  return `Once ${pad(r.Once.year)}-${pad(r.Once.month)}-${pad(r.Once.day)}`;
}

async function cycleImportance(t: Todo) {
  const id = selectedDeviceId.value;
  if (id == null) return;
  const next: "low" | "medium" | "high" =
    t.importance === "low" ? "medium" : t.importance === "medium" ? "high" : "low";
  const r = await api.updateTodo(token.value, id, t.id, {
    text: t.text,
    done: t.done,
    importance: next,
    due_date: t.due_date,
    repeat: t.repeat,
  });
  if (!r.ok) toast(r.error);
  else await loadContent();
}

async function deleteTodo(todoId: number) {
  const id = selectedDeviceId.value;
  if (id == null) return;
  const r = await api.deleteTodo(token.value, id, todoId);
  if (!r.ok) toast(r.error);
  else await loadContent();
}

async function clearTodos() {
  const id = selectedDeviceId.value;
  if (id == null || !confirm("Clear all todos?")) return;
  const r = await api.clearTodos(token.value, id);
  if (!r.ok) toast(r.error);
  else {
    await loadContent();
    toast("All todos cleared");
  }
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

onMounted(() => {
  if (token.value) connect();
});
</script>

<template>
  <main>
    <header class="masthead">
      <div>
        <div class="eyebrow">Device cloud</div>
        <h1>Inkpaper Console</h1>
        <p>Manage device content and prepare the next sync.</p>
      </div>
      <span class="status" :class="{ ok: connected }">{{ connected ? "● Server connected" : "○ Not connected" }}</span>
    </header>

    <div class="layout">
      <aside class="sidebar">
        <section class="card">
          <div class="card-head"><h2>Server access</h2></div>
          <div class="stack">
            <input v-model="token" type="password" placeholder="Admin token" @keyup.enter="connect" />
            <button class="primary" @click="connect">Connect to server</button>
          </div>
        </section>

        <section class="card">
          <div class="card-head">
            <h2>Device</h2>
            <span class="count">{{ devices.length }}</span>
          </div>
          <div class="stack">
            <select :value="selectedDeviceId ?? ''" @change="(e) => pickDevice((e.target as HTMLSelectElement).value)">
              <option value="" disabled>{{ devices.length ? "Select a device" : "Connect first" }}</option>
              <option v-for="d in devices" :key="d.id" :value="d.id">{{ d.name }}</option>
            </select>
            <div class="row">
              <button class="quiet" @click="loadDevices">Refresh</button>
              <button class="danger" :disabled="selectedDeviceId == null" @click="deleteDevice(selectedDeviceId!)">Delete device</button>
            </div>
          </div>

          <div class="divider"></div>
          <label>
            Register another device
            <div class="row">
              <input v-model="newDeviceName" class="grow" placeholder="Device name" @keyup.enter="registerDevice" />
              <button class="primary" @click="registerDevice">Register</button>
            </div>
          </label>
          <div v-if="newDeviceToken" class="token-box">
            Device token (shown once): <code>{{ newDeviceToken }}</code>
            <div class="row" style="margin-top: 8px">
              <button class="quiet" @click="copyNewDeviceToken">Copy</button>
            </div>
          </div>
        </section>
      </aside>

      <section>
        <div class="device-context">
          <template v-if="selectedDevice">
            Editing <strong>{{ selectedDevice.name }}</strong> · changes become available to the device on its next sync.
          </template>
          <template v-else>Select a device to manage the content it receives on the next sync.</template>
        </div>

        <div class="content-grid">
          <section class="card">
            <div class="card-head">
              <h2>Alarms</h2>
              <span class="count">{{ alarms.length }}</span>
            </div>
            <div class="row">
              <label>Hour<input v-model.number="alarmHour" type="number" min="0" max="23" style="width: 85px" /></label>
              <label>Minute<input v-model.number="alarmMinute" type="number" min="0" max="59" style="width: 85px" /></label>
              <label class="grow">Label<input v-model="alarmLabel" placeholder="Wake up" /></label>
              <button class="primary" :disabled="selectedDeviceId == null" @click="addAlarm">Add</button>
            </div>
            <div class="row" style="margin-top: 8px">
              <label>Repeat
                <select v-model="alarmRepeatKind">
                  <option value="Daily">Daily</option>
                  <option value="Weekly">Weekly</option>
                  <option value="Monthly">Monthly</option>
                  <option value="Once">Once</option>
                </select>
              </label>
              <label v-if="alarmRepeatKind === 'Weekly' || alarmRepeatKind === 'Monthly'">Days (comma-sep)
                <input v-model="alarmRepeatDays" :placeholder="alarmRepeatKind === 'Weekly' ? '0,2,4 (0=Sun)' : '1,15'" style="width: 130px" />
              </label>
              <label v-if="alarmRepeatKind === 'Once'">Date<input v-model="alarmOnceDate" type="date" /></label>
            </div>
            <div class="list">
              <div v-if="alarms.length === 0" class="empty">{{ selectedDeviceId == null ? "No device selected" : "No alarms scheduled" }}</div>
              <div v-for="a in alarms" :key="a.id" class="item">
                <span class="text">
                  <b>{{ pad(a.hour) }}:{{ pad(a.minute) }}</b><br />
                  <span class="meta">{{ a.label || "No label" }} · {{ repeatLabel(a.repeat) }} · {{ a.enabled ? "Enabled" : "Disabled" }}</span>
                </span>
                <button class="danger" @click="deleteAlarm(a.id)">Delete</button>
              </div>
            </div>
            <button class="danger" style="margin-top: 14px" :disabled="selectedDeviceId == null" @click="clearAlarms">Clear alarms</button>
          </section>

          <section class="card">
            <div class="card-head">
              <h2>Todos</h2>
              <span class="count">{{ todos.length }}</span>
            </div>
            <div class="row">
              <input v-model="todoText" class="grow" placeholder="Test todo" @keyup.enter="addTodo" />
              <select v-model="todoImportance" title="Importance">
                <option value="low">L</option>
                <option value="medium">M</option>
                <option value="high">H</option>
              </select>
              <input v-model="todoDue" type="date" title="Due date" @keyup.enter="addTodo" />
              <select v-model="todoRepeatKind" title="Repeat">
                <option value="">Once</option>
                <option value="Daily">Daily</option>
                <option value="Weekly">Weekly</option>
                <option value="Monthly">Monthly</option>
              </select>
              <input v-if="todoRepeatKind === 'Weekly' || todoRepeatKind === 'Monthly'" v-model="todoRepeatDays" :placeholder="todoRepeatKind === 'Weekly' ? '0,2,4' : '1,15'" style="width: 70px" />
              <button class="primary" :disabled="selectedDeviceId == null" @click="addTodo">Add</button>
            </div>
            <div class="list">
              <div v-if="todos.length === 0" class="empty">{{ selectedDeviceId == null ? "No device selected" : "No todos waiting" }}</div>
              <div v-for="t in todos" :key="t.id" class="item">
                <span class="text">
                  {{ t.done ? "✓ " : "" }}{{ t.text }}
                  <template v-if="t.done"><br /><span class="meta">Completed</span></template>
                  <template v-else-if="t.due_date || t.repeat"><br /><span class="meta">Due {{ t.due_date ? formatDue(t) : "—" }} · {{ repeatLabel(t.repeat) }}</span></template>
                </span>
                <button :title="'Importance: ' + t.importance" @click="cycleImportance(t)">{{ todoBadge(t.importance) }}</button>
                <button class="danger" @click="deleteTodo(t.id)">Delete</button>
              </div>
            </div>
            <button class="danger" style="margin-top: 14px" :disabled="selectedDeviceId == null" @click="clearTodos">Clear todos</button>
          </section>
        </div>
      </section>
    </div>
  </main>
  <div class="toast" :class="{ show: toastVisible }">{{ toastText }}</div>
</template>
