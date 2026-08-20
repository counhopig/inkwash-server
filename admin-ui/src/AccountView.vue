<script setup lang="ts">
import { inject, ref } from "vue";
import * as api from "./lib/api";
import { uiKey } from "./lib/ui";

const emit = defineEmits<{
  (e: "back"): void;
  (e: "logout"): void;
}>();

const ui = inject(uiKey)!;

const oldPassword = ref("");
const newPassword = ref("");
const newPasswordConfirm = ref("");

async function changePassword() {
  if (ui.session.value?.kind !== "account") return;
  if (!newPassword.value) {
    ui.toast("Enter a new password", "error");
    return;
  }
  if (newPassword.value !== newPasswordConfirm.value) {
    ui.toast("New passwords do not match", "error");
    return;
  }
  const ok = await ui.run("change-password", () =>
    api.changePassword(ui.authToken(), oldPassword.value, newPassword.value),
  );
  if (ok === null) return;
  oldPassword.value = "";
  newPassword.value = "";
  newPasswordConfirm.value = "";
  ui.toast("Password changed");
}
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

      <section v-if="ui.session.value?.kind === 'account'" class="card" style="--i: 4">
        <div class="card-head">
          <h2><span class="index">02</span>Password</h2>
        </div>
        <form class="stack" @submit.prevent="changePassword">
          <label>
            Current password
            <input v-model="oldPassword" type="password" autocomplete="current-password" :disabled="ui.isBusy('change-password')" />
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

      <section v-else class="card" style="--i: 4">
        <div class="card-head">
          <h2><span class="index">02</span>Admin</h2>
        </div>
        <p class="account-line">
          You are signed in with the server admin token. The token is set in the server
          configuration (<code>ADMIN_TOKEN</code>), so there is no password to change here.
        </p>
      </section>
    </div>
  </div>
</template>
