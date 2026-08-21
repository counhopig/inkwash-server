<script setup lang="ts">
import { inject, onMounted, ref, watch } from "vue";
import * as api from "./lib/api";
import { uiKey } from "./lib/ui";
import type { Alarm, Channel, Device, InboxItem, Repeat, Todo, TodoDue } from "./lib/types";

const props = defineProps<{ device: Device }>();

const emit = defineEmits<{
  (e: "back"): void;
  (e: "deleted"): void;
}>();

const ui = inject(uiKey)!;

const alarms = ref<Alarm[]>([]);
const todos = ref<Todo[]>([]);
const channels = ref<Channel[]>([]);
const inbox = ref<InboxItem[]>([]);
const pendingToken = ref<{ channel: Channel; token: string; url: string } | null>(null);
const newChannelName = ref("");
const newChannelKind = ref<"webhook" | "caldav_basic">("webhook");

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

/** Sequence guard so a slow stale response can't clobber a newer one. */
let loadSeq = 0;

async function loadContent() {
  const id = props.device.id;
  const seq = ++loadSeq;
  const [a, t, c, i] = await Promise.all([
    handle(api.listAlarms(ui.authToken(), id)),
    handle(api.listTodos(ui.authToken(), id)),
    handle(api.listChannels(ui.authToken(), id)),
    handle(api.listInbox(ui.authToken(), id)),
  ]);
  if (seq !== loadSeq) return;
  if (a !== null) alarms.value = a;
  if (t !== null) todos.value = t;
  if (c !== null) channels.value = c;
  if (i !== null) inbox.value = i;
}

async function handle<T>(pending: Promise<api.Result<T>>): Promise<T | null> {
  const r = await pending;
  if (r.ok) return r.value;
  if (r.status === 401) {
    ui.onUnauthorized();
    return null;
  }
  ui.toast(r.error || `HTTP ${r.status}`, "error");
  return null;
}

async function deleteDevice() {
  if (!(await ui.confirmDialog("Delete this device and all its content?"))) return;
  const ok = await ui.run("delete-device", () => api.deleteDevice(ui.authToken(), props.device.id));
  if (ok === null) return;
  emit("deleted");
}

// --- Alarms ----------------------------------------------------------------

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

async function addAlarm() {
  const created = await ui.run("add-alarm", () =>
    api.createAlarm(ui.authToken(), props.device.id, {
      hour: alarmHour.value,
      minute: alarmMinute.value,
      repeat: buildRepeat(alarmRepeatKind.value, alarmRepeatDays.value, alarmOnceDate.value),
      enabled: true,
      label: alarmLabel.value.trim(),
    }),
  );
  if (created === null) return;
  alarmLabel.value = "";
  await loadContent();
  ui.toast("Alarm added");
}

async function toggleAlarm(a: Alarm) {
  const ok = await ui.run("toggle-alarm", () =>
    api.updateAlarm(ui.authToken(), props.device.id, a.id, {
      hour: a.hour,
      minute: a.minute,
      repeat: a.repeat,
      enabled: !a.enabled,
      label: a.label,
    }),
  );
  if (ok === null) return;
  await loadContent();
}

async function deleteAlarm(alarmId: number) {
  const ok = await ui.run("delete-alarm", () =>
    api.deleteAlarm(ui.authToken(), props.device.id, alarmId),
  );
  if (ok === null) return;
  await loadContent();
}

async function clearAlarms() {
  if (!(await ui.confirmDialog("Clear all alarms?"))) return;
  const ok = await ui.run("clear-alarms", () => api.clearAlarms(ui.authToken(), props.device.id));
  if (ok === null) return;
  await loadContent();
  ui.toast("All alarms cleared");
}

// --- Todos -----------------------------------------------------------------

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
  const text = todoText.value.trim();
  if (!text) {
    ui.toast("Enter todo text", "error");
    return;
  }
  const due = todoDue.value.trim();
  const dueDate = due ? parseDueDate(due) : null;
  if (due && !dueDate) {
    ui.toast("Due date must be YYYY-MM-DD", "error");
    return;
  }
  const repeat = todoRepeatKind.value
    ? buildRepeat(todoRepeatKind.value, todoRepeatDays.value, "")
    : null;
  const created = await ui.run("add-todo", () =>
    api.createTodo(ui.authToken(), props.device.id, {
      text,
      done: false,
      importance: todoImportance.value,
      due_date: dueDate,
      repeat,
    }),
  );
  if (created === null) return;
  todoText.value = "";
  todoDue.value = "";
  todoRepeatKind.value = "";
  todoRepeatDays.value = "";
  await loadContent();
  ui.toast("Todo added");
}

async function toggleTodo(t: Todo) {
  const ok = await ui.run("toggle-todo", () =>
    api.updateTodo(ui.authToken(), props.device.id, t.id, {
      text: t.text,
      done: !t.done,
      importance: t.importance,
      due_date: t.due_date,
      repeat: t.repeat,
    }),
  );
  if (ok === null) return;
  await loadContent();
}

async function cycleImportance(t: Todo) {
  const next: "low" | "medium" | "high" =
    t.importance === "low" ? "medium" : t.importance === "medium" ? "high" : "low";
  const ok = await ui.run("toggle-importance", () =>
    api.updateTodo(ui.authToken(), props.device.id, t.id, {
      text: t.text,
      done: t.done,
      importance: next,
      due_date: t.due_date,
      repeat: t.repeat,
    }),
  );
  if (ok === null) return;
  await loadContent();
}

async function deleteTodo(todoId: number) {
  const ok = await ui.run("delete-todo", () =>
    api.deleteTodo(ui.authToken(), props.device.id, todoId),
  );
  if (ok === null) return;
  await loadContent();
}

async function clearTodos() {
  if (!(await ui.confirmDialog("Clear all todos?"))) return;
  const ok = await ui.run("clear-todos", () => api.clearTodos(ui.authToken(), props.device.id));
  if (ok === null) return;
  await loadContent();
  ui.toast("All todos cleared");
}

// --- Channels & inbox ------------------------------------------------------

async function addChannel() {
  const name = newChannelName.value.trim();
  if (!name) {
    ui.toast("Enter a channel name", "error");
    return;
  }
  if (newChannelKind.value === "caldav_basic") {
    ui.toast("CalDAV is not implemented yet", "error");
    return;
  }
  const created = await ui.run("add-channel", () =>
    api.createChannel(ui.authToken(), props.device.id, "webhook", name),
  );
  if (created === null) return;
  if (created.token) {
    pendingToken.value = {
      channel: created.channel,
      token: created.token,
      url: created.delivery_url ?? "",
    };
  }
  newChannelName.value = "";
  await loadContent();
  if (pendingToken.value) ui.toast("Webhook channel created — copy its token now (shown once)");
}

async function removeChannel(c: Channel) {
  if (!(await ui.confirmDialog(`Delete channel "${c.name}" and its messages?`))) return;
  const ok = await ui.run("delete-channel", () =>
    api.deleteChannel(ui.authToken(), props.device.id, c.id),
  );
  if (ok === null) return;
  await loadContent();
}

async function rotateToken(c: Channel) {
  const res = await ui.run("rotate-token", () =>
    api.rotateChannelToken(ui.authToken(), props.device.id, c.id),
  );
  if (res === null) return;
  pendingToken.value = {
    channel: c,
    token: res.token,
    url: `/api/channels/${c.id}/messages`,
  };
  await loadContent();
  ui.toast("Token rotated — the old one no longer works");
}

function copyText(text: string) {
  void navigator.clipboard?.writeText(text);
  ui.toast("Copied to clipboard");
}

function copyToken() {
  if (pendingToken.value) copyText(pendingToken.value.token);
}

async function deleteInboxItem(seq: number) {
  const ok = await ui.run("delete-inbox", () =>
    api.deleteInboxItem(ui.authToken(), props.device.id, seq),
  );
  if (ok === null) return;
  await loadContent();
}

async function clearInbox() {
  if (!(await ui.confirmDialog("Delete all read inbox messages?"))) return;
  const ok = await ui.run("clear-inbox", () => api.clearInbox(ui.authToken(), props.device.id));
  if (ok === null) return;
  await loadContent();
}

// --- Display helpers ---------------------------------------------------------

function pad(n: number): string {
  return String(n).padStart(2, "0");
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

onMounted(loadContent);
watch(() => props.device.id, loadContent);
</script>

<template>
  <div class="view">
    <div class="detail-head">
      <button class="quiet back-link" type="button" @click="emit('back')">← Dashboard</button>
      <div class="detail-title">
        <span class="ctx-mark" aria-hidden="true">▸</span>
        Editing <strong>{{ device.name }}</strong>
        <span class="meta">· changes reach the device on its next sync</span>
      </div>
      <button
        class="danger"
        type="button"
        :disabled="ui.isBusy('delete-device')"
        @click="deleteDevice"
      >
        {{ ui.isBusy("delete-device") ? "Deleting…" : "Delete device" }}
      </button>
    </div>

    <div class="content-grid">
      <form class="card" style="--i: 3" @submit.prevent="addAlarm">
        <div class="card-head">
          <h2><span class="index">01</span>Alarms</h2>
          <span class="count">{{ alarms.length }}</span>
        </div>
        <div class="row">
          <label>Hour<input v-model.number="alarmHour" type="number" min="0" max="23" style="width: 85px" :disabled="ui.isBusy('add-alarm')" /></label>
          <label>Minute<input v-model.number="alarmMinute" type="number" min="0" max="59" style="width: 85px" :disabled="ui.isBusy('add-alarm')" /></label>
          <label class="grow">Label<input v-model="alarmLabel" placeholder="Wake up" :disabled="ui.isBusy('add-alarm')" /></label>
          <button class="primary" type="submit" :disabled="ui.isBusy('add-alarm')">
            {{ ui.isBusy("add-alarm") ? "Adding…" : "Add" }}
          </button>
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
          <div v-if="alarms.length === 0" class="empty">No alarms scheduled</div>
          <div v-for="a in alarms" :key="a.id" class="item" :class="{ off: !a.enabled }">
            <button
              class="tick"
              type="button"
              :class="{ on: a.enabled }"
              :title="a.enabled ? 'Disable alarm' : 'Enable alarm'"
              :aria-label="(a.enabled ? 'Disable' : 'Enable') + ' alarm at ' + pad(a.hour) + ':' + pad(a.minute)"
              :disabled="ui.isBusy('toggle-alarm')"
              @click="toggleAlarm(a)"
            ></button>
            <span class="text">
              <b>{{ pad(a.hour) }}:{{ pad(a.minute) }}</b>
              <span class="meta">
                {{ a.label || "No label" }} · {{ repeatLabel(a.repeat) }} · {{ a.enabled ? "enabled" : "muted" }}
              </span>
            </span>
            <button class="danger" type="button" :disabled="ui.isBusy('delete-alarm')" @click="deleteAlarm(a.id)">Delete</button>
          </div>
        </div>
        <button class="danger" type="button" style="margin-top: 14px" @click="clearAlarms">
          Clear alarms
        </button>
      </form>

      <form class="card" style="--i: 4" @submit.prevent="addTodo">
        <div class="card-head">
          <h2><span class="index">02</span>Todos</h2>
          <span class="count">{{ todos.length }}</span>
        </div>
        <div class="row">
          <input v-model="todoText" class="grow" placeholder="Test todo" :disabled="ui.isBusy('add-todo')" />
          <select v-model="todoImportance" title="Importance">
            <option value="low">L</option>
            <option value="medium">M</option>
            <option value="high">H</option>
          </select>
          <input v-model="todoDue" type="date" title="Due date" />
          <select v-model="todoRepeatKind" title="Repeat">
            <option value="">Once</option>
            <option value="Daily">Daily</option>
            <option value="Weekly">Weekly</option>
            <option value="Monthly">Monthly</option>
          </select>
          <input v-if="todoRepeatKind === 'Weekly' || todoRepeatKind === 'Monthly'" v-model="todoRepeatDays" :placeholder="todoRepeatKind === 'Weekly' ? '0,2,4' : '1,15'" style="width: 70px" />
          <button class="primary" type="submit" :disabled="ui.isBusy('add-todo')">
            {{ ui.isBusy("add-todo") ? "Adding…" : "Add" }}
          </button>
        </div>
        <div class="list">
          <div v-if="todos.length === 0" class="empty">No todos waiting</div>
          <div v-for="t in todos" :key="t.id" class="item" :class="{ done: t.done }">
            <button
              class="tick"
              type="button"
              :class="{ on: t.done }"
              :title="t.done ? 'Mark not done' : 'Mark done'"
              :aria-label="(t.done ? 'Mark not done' : 'Mark done') + ': ' + t.text"
              :disabled="ui.isBusy('toggle-todo')"
              @click="toggleTodo(t)"
            ></button>
            <span class="text">
              {{ t.text }}
              <template v-if="t.done"><br /><span class="meta">Completed</span></template>
              <template v-else-if="t.due_date || t.repeat"><br /><span class="meta">Due {{ t.due_date ? formatDue(t) : "—" }} · {{ repeatLabel(t.repeat) }}</span></template>
            </span>
            <button class="imp" type="button" :class="t.importance" :title="'Importance: ' + t.importance" :disabled="ui.isBusy('toggle-importance')" @click="cycleImportance(t)">
              {{ todoBadge(t.importance) }}
            </button>
            <button class="danger" type="button" :disabled="ui.isBusy('delete-todo')" @click="deleteTodo(t.id)">Delete</button>
          </div>
        </div>
        <button class="danger" type="button" style="margin-top: 14px" @click="clearTodos">
          Clear todos
        </button>
      </form>
    </div>

    <form class="card" style="--i: 5; margin-top: 16px" @submit.prevent="addChannel">
      <div class="card-head">
        <h2><span class="index">03</span>Channels</h2>
        <span class="count">{{ channels.length }}</span>
      </div>
      <p class="hint">
        External sources that push notifications to this device. A webhook channel
        gives you a token so CI, agents or scripts can deliver messages.
      </p>
      <div class="row">
        <input v-model="newChannelName" class="grow" placeholder="Channel name (e.g. CI)" :disabled="ui.isBusy('add-channel')" />
        <select v-model="newChannelKind">
          <option value="webhook">Webhook</option>
          <option value="caldav_basic" disabled>CalDAV (soon)</option>
        </select>
        <button class="primary" type="submit" :disabled="ui.isBusy('add-channel')">
          {{ ui.isBusy("add-channel") ? "Creating…" : "Create" }}
        </button>
      </div>
      <div class="list">
        <div v-if="channels.length === 0" class="empty">No channels yet</div>
        <div v-for="c in channels" :key="c.id" class="item" :class="{ off: !c.enabled }">
          <span class="text">
            <b>{{ c.name }}</b>
            <span class="meta">{{ c.kind }} · {{ c.enabled ? "enabled" : "disabled" }} · token {{ c.token_prefix }}…</span>
          </span>
          <button class="imp" type="button" title="Rotate token (old one stops working)" :disabled="ui.isBusy('rotate-token')" @click="rotateToken(c)">Rotate</button>
          <button class="danger" type="button" :disabled="ui.isBusy('delete-channel')" @click="removeChannel(c)">Delete</button>
        </div>
      </div>
    </form>

    <div class="card" style="--i: 6; margin-top: 16px">
      <div class="card-head">
        <h2><span class="index">04</span>Inbox</h2>
        <span class="count">{{ inbox.length }}</span>
      </div>
      <div class="list">
        <div v-if="inbox.length === 0" class="empty">No messages</div>
        <div v-for="m in inbox" :key="m.id" class="item" :class="{ done: m.read }">
          <span class="text">
            <b>{{ m.title }}</b>
            <span class="meta">{{ m.kind }} · {{ m.read ? "read" : "unread" }}{{ m.when ? " · " + new Date(m.when * 1000).toLocaleString() : "" }}</span>
            <template v-if="m.body"><br /><span class="meta">{{ m.body }}</span></template>
          </span>
          <button class="danger" type="button" :disabled="ui.isBusy('delete-inbox')" @click="deleteInboxItem(m.id)">Delete</button>
        </div>
      </div>
      <button class="danger" type="button" style="margin-top: 14px" @click="clearInbox">
        Clear read messages
      </button>
    </div>

    <div v-if="pendingToken" class="modal-backdrop" @click.self="pendingToken = null">
      <div class="modal">
        <h3>Webhook token (shown once)</h3>
        <p class="hint">Copy this token now — it is never shown again. Anyone holding it can post messages to this channel.</p>
        <pre class="token">{{ pendingToken.token }}</pre>
        <p class="hint">Delivery URL:</p>
        <pre class="token">{{ pendingToken.url }}</pre>
        <div class="row">
          <button class="primary" type="button" @click="copyToken">Copy token</button>
          <button class="quiet" type="button" @click="pendingToken = null">Close</button>
        </div>
      </div>
    </div>
  </div>
</template>
