<script setup lang="ts">
import { inject, onMounted, ref } from "vue";
import * as api from "./lib/api";
import { uiKey } from "./lib/ui";
import type { AccountSummary } from "./lib/types";

const emit = defineEmits<{
  (e: "back"): void;
  (e: "logout"): void;
}>();

const ui = inject(uiKey)!;

const accounts = ref<AccountSummary[]>([]);
const accountsLoaded = ref(false);

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

async function loadAccounts() {
  if (ui.session.value?.kind !== "admin") return;
  const list = await handle(api.listAccounts(ui.authToken()));
  if (list !== null) {
    accounts.value = list;
    accountsLoaded.value = true;
  }
}

async function resetPassword(a: AccountSummary) {
  const next = await ui.promptDialog(
    `New password for “${a.username}” (at least 8 characters)`,
    "New password",
  );
  if (next === null) return;
  const ok = await ui.run(`reset-${a.id}`, () =>
    api.resetAccountPassword(ui.authToken(), a.id, next),
  );
  if (ok === null) return;
  ui.toast(`Password reset for ${a.username}`);
}

async function removeAccount(a: AccountSummary) {
  const confirmed = await ui.confirmDialog(
    `Delete account “${a.username}” and all its devices and content?`,
  );
  if (!confirmed) return;
  const ok = await ui.run(`delete-${a.id}`, () => api.deleteAccount(ui.authToken(), a.id));
  if (ok === null) return;
  accounts.value = accounts.value.filter((x) => x.id !== a.id);
  ui.toast(`Account ${a.username} deleted`);
}

async function changePassword() {
  if (ui.session.value?.kind !== "account") return;
  const oldPassword = currentPassword.value;
  if (!newPassword.value) {
    ui.toast("Enter a new password", "error");
    return;
  }
  if (newPassword.value !== newPasswordConfirm.value) {
    ui.toast("New passwords do not match", "error");
    return;
  }
  const ok = await ui.run("change-password", () =>
    api.changePassword(ui.authToken(), oldPassword, newPassword.value),
  );
  if (ok === null) return;
  currentPassword.value = "";
  newPassword.value = "";
  newPasswordConfirm.value = "";
  ui.toast("Password changed");
}

function formatDate(unixSec: number): string {
  const d = new Date(unixSec * 1000);
  return d.toLocaleDateString();
}

const currentPassword = ref("");
const newPassword = ref("");
const newPasswordConfirm = ref("");

onMounted(loadAccounts);
</script>

<template>
  <div class="view">
    <div class="detail-head">
      <button class="quiet back-link" type="button" @click="emit('back')">← Dashboard</button>
      <div class="detail-title">
        <span class="ctx-mark" aria-hidden="true">▸</span>
        Account settings
      </div>
    </div>

    <div class="account-grid">
      <section class="card" style="--i: 3">
        <div class="card-head">
          <h2><span class="index">01</span>Session</h2>
        </div>
        <dl class="kv">
          <div><dt>Signed in as</dt><dd>{{ ui.session.value?.username }}</dd></div>
          <div>
            <dt>Credential</dt>
            <dd>{{ ui.session.value?.kind === "admin" ? "Admin token" : "Account" }}</dd>
          </div>
        </dl>
        <button class="danger" type="button" style="margin-top: 18px" @click="emit('logout')">
          Sign out
        </button>
      </section>

      <template v-if="ui.session.value?.kind === 'account'">
        <section class="card" style="--i: 4">
          <div class="card-head">
            <h2><span class="index">02</span>Password</h2>
          </div>
          <form class="stack" @submit.prevent="changePassword">
            <label>
              Current password
              <input v-model="currentPassword" type="password" autocomplete="current-password" :disabled="ui.isBusy('change-password')" />
            </label>
            <label>
              New password
              <input v-model="newPassword" type="password" autocomplete="new-password" :disabled="ui.isBusy('change-password')" />
            </label>
            <label>
              Confirm new password
              <input v-model="newPasswordConfirm" type="password" autocomplete="new-password" :disabled="ui.isBusy('change-password')" />
            </label>
            <button class="primary" type="submit" :disabled="ui.isBusy('change-password')">
              {{ ui.isBusy("change-password") ? "Updating…" : "Change password" }}
            </button>
          </form>
        </section>
      </template>

      <template v-else>
        <section class="card users-card" style="--i: 4">
          <div class="card-head">
            <h2><span class="index">02</span>Users</h2>
            <span class="count">{{ accounts.length }}</span>
          </div>
          <div class="row" style="margin-bottom: 12px">
            <button class="quiet" type="button" @click="loadAccounts">Refresh</button>
          </div>
          <div v-if="!accountsLoaded" class="empty">Loading users…</div>
          <div v-else-if="accounts.length === 0" class="empty">No console accounts yet.</div>
          <div v-else class="user-list">
            <div v-for="a in accounts" :key="a.id" class="user-row">
              <span class="text">
                <b>{{ a.username }}</b><br />
                <span class="meta">{{ a.device_count }} device{{ a.device_count === 1 ? "" : "s" }} · {{ a.session_count }} session{{ a.session_count === 1 ? "" : "s" }} · since {{ formatDate(a.created_at) }}</span>
              </span>
              <button class="quiet" type="button" :disabled="ui.isBusy(`reset-${a.id}`)" @click="resetPassword(a)">
                Reset password
              </button>
              <button class="danger" type="button" :disabled="ui.isBusy(`delete-${a.id}`)" @click="removeAccount(a)">
                Delete
              </button>
            </div>
          </div>
        </section>
      </template>
    </div>
  </div>
</template>
