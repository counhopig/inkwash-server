<script setup lang="ts">
import { ref } from "vue";
import * as api from "./lib/api";
import type { Session } from "./lib/storage";

const emit = defineEmits<{ (e: "authenticated", session: Session): void }>();

const mode = ref<"login" | "register">("login");
const username = ref("");
const password = ref("");
const passwordConfirm = ref("");
const adminToken = ref("");
const error = ref("");
const busy = ref<"account" | "admin" | null>(null);

function submitAccount() {
  error.value = "";
  const u = username.value.trim();
  if (!u || !password.value) {
    error.value = "Enter a username and password";
    return;
  }
  if (mode.value === "register" && password.value !== passwordConfirm.value) {
    error.value = "Passwords do not match";
    return;
  }
  if (mode.value === "register" && password.value.length < 8) {
    error.value = "Password must be at least 8 characters";
    return;
  }
  busy.value = "account";
  const req =
    mode.value === "login"
      ? api.loginAccount(u, password.value)
      : api.registerAccount(u, password.value);
  req.then((r) => {
    busy.value = null;
    if (!r.ok) {
      error.value = r.error || `HTTP ${r.status}`;
      return;
    }
    emit("authenticated", {
      kind: "account",
      token: r.value.token,
      username: r.value.username,
    });
  });
}

function submitAdmin() {
  error.value = "";
  const t = adminToken.value.trim();
  if (!t) {
    error.value = "Enter the admin token";
    return;
  }
  busy.value = "admin";
  api.listDevices(t).then((r) => {
    busy.value = null;
    if (!r.ok) {
      error.value = r.status === 401 ? "Invalid admin token" : r.error;
      return;
    }
    emit("authenticated", { kind: "admin", token: t, username: "admin" });
  });
}

function switchMode(next: "login" | "register") {
  mode.value = next;
  error.value = "";
}
</script>

<template>
  <main class="login-page">
    <header class="masthead login-masthead">
      <div class="masthead-title">
        <div class="eyebrow">Device cloud · admin console</div>
        <h1>Inkwash Console</h1>
        <p>Sign in to manage device content and prepare the next sync.</p>
      </div>
    </header>

    <div class="login-wrap">
      <section class="card login-card">
        <div class="tabs" role="tablist" aria-label="Authentication">
          <button
            :class="{ active: mode === 'login' }"
            role="tab"
            :aria-selected="mode === 'login'"
            :disabled="busy !== null"
            @click="switchMode('login')"
          >
            Sign in
          </button>
          <button
            :class="{ active: mode === 'register' }"
            role="tab"
            :aria-selected="mode === 'register'"
            :disabled="busy !== null"
            @click="switchMode('register')"
          >
            Create account
          </button>
        </div>

        <form class="stack" @submit.prevent="submitAccount">
          <label>
            Username
            <input
              v-model="username"
              autocomplete="username"
              placeholder="e.g. alice"
              :disabled="busy !== null"
            />
          </label>
          <label>
            Password
            <input
              v-model="password"
              type="password"
              :autocomplete="mode === 'login' ? 'current-password' : 'new-password'"
              :placeholder="mode === 'register' ? 'At least 8 characters' : '••••••••'"
              :disabled="busy !== null"
            />
          </label>
          <label v-if="mode === 'register'">
            Confirm password
            <input
              v-model="passwordConfirm"
              type="password"
              autocomplete="new-password"
              :disabled="busy !== null"
            />
          </label>
          <button class="primary" type="submit" :disabled="busy !== null">
            {{ busy === "account" ? "Working…" : mode === "login" ? "Sign in" : "Create account" }}
          </button>
        </form>

        <div class="divider"></div>

        <details class="admin-toggle" @toggle="error = ''">
          <summary>Server owner? Sign in with the admin token</summary>
          <form class="stack" @submit.prevent="submitAdmin">
            <label>
              Admin token
              <input
                v-model="adminToken"
                type="password"
                autocomplete="off"
                placeholder="ADMIN_TOKEN from server config"
                :disabled="busy !== null"
              />
            </label>
            <button type="submit" :disabled="busy !== null">
              {{ busy === "admin" ? "Checking…" : "Connect with admin token" }}
            </button>
          </form>
        </details>

        <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      </section>

      <p class="login-hint">
        <span aria-hidden="true">◌</span> Same-origin console · tokens are kept on this device
      </p>
    </div>
  </main>
</template>
