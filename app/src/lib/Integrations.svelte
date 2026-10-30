<script lang="ts">
  import { onMount } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import Icon from './Icon.svelte';
  import {
    api,
    errText,
    type ApiToken,
    type McpInfo,
    type ReminderWebhook,
    type SyncStatus,
    type TelegramStatus,
    type WebhookTestResult,
  } from './api';

  interface Props {
    status: SyncStatus | null;
    onClose: () => void;
    onSync: () => void;
  }
  let { status, onClose, onSync }: Props = $props();

  let tg = $state<TelegramStatus | null>(null);
  let mcp = $state<McpInfo | null>(null);
  let tokens = $state<ApiToken[]>([]);
  let created = $state<{ name: string; token: string } | null>(null);
  let tokenName = $state('Claude');
  let botToken = $state('');
  let botError = $state('');
  let waiting = $state(false);
  let copied = $state('');
  let busy = $state(false);
  let error = $state('');

  let hook = $state<ReminderWebhook | null>(null);
  let hookUrl = $state('');
  let hookSecret = $state('');
  let hookError = $state('');
  let hookTest = $state<WebhookTestResult | null>(null);
  let hookBusy = $state<'' | 'save' | 'test' | 'remove'>('');

  const connected = $derived(!!status?.configured);
  const e2e = $derived(status?.mode === 'e2e');

  async function run(fn: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await fn();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  async function refreshTelegram() {
    try {
      tg = await api.telegramStatus();
      if (tg.linked) waiting = false;
    } catch (e) {
      error = errText(e);
    }
  }

  onMount(() => {
    api.mcpInfo().then((m) => (mcp = m));
    if (!connected) return;
    refreshTelegram();
    api.apiTokens().then((t) => (tokens = t)).catch(() => {});
    api.reminderWebhook().then(showHook).catch(() => {});
  });

  // After the t.me link is opened, wait for the user to press Start.
  $effect(() => {
    if (!waiting) return;
    let n = 0;
    const timer = setInterval(() => {
      if (++n > 60) waiting = false;
      else refreshTelegram();
    }, 2500);
    return () => clearInterval(timer);
  });

  const linkTelegram = () =>
    run(async () => {
      const link = await api.telegramLink();
      await openUrl(link.url);
      waiting = true;
    });

  const setChanges = (on: boolean) => run(async () => void (tg = await api.telegramSetChanges(on)));
  const unlink = () => run(async () => void (tg = await api.telegramUnlink()));
  // Its error goes right under the token field, not at the bottom of the dialog.
  async function setBot() {
    busy = true;
    botError = '';
    try {
      tg = await api.telegramSetBot(botToken.trim());
      botToken = '';
    } catch (e) {
      botError = errText(e);
    } finally {
      busy = false;
    }
  }
  const removeBot = () => run(async () => void (tg = await api.telegramRemoveBot()));
  const setShare = (on: boolean) => run(() => api.setShareReminders(on));

  const hookDirty = $derived(hookUrl.trim() !== (hook?.url ?? '') || hookSecret.trim() !== '');

  function showHook(h: ReminderWebhook) {
    hook = h;
    hookUrl = h.url ?? '';
    hookSecret = '';
  }

  // Errors go right under the fields, like the bot token's.
  async function hookAction(kind: 'save' | 'test' | 'remove', fn: () => Promise<void>) {
    hookBusy = kind;
    hookError = '';
    try {
      await fn();
    } catch (e) {
      hookError = errText(e);
    } finally {
      hookBusy = '';
    }
  }

  const saveHook = () => hookAction('save', async () => {
    showHook(await api.setReminderWebhook(hookUrl.trim(), hookSecret.trim()));
    hookTest = null;
  });
  // Unsaved changes are saved first: the test goes to what the server has.
  const testHook = () => hookAction('test', async () => {
    hookTest = null;
    if (hookDirty) showHook(await api.setReminderWebhook(hookUrl.trim(), hookSecret.trim()));
    hookTest = await api.testReminderWebhook();
    hook = await api.reminderWebhook();
  });
  const removeHook = () => hookAction('remove', async () => {
    showHook(await api.deleteReminderWebhook());
    hookTest = null;
  });

  const testText = (t: WebhookTestResult) => {
    const body = t.body.trim().slice(0, 300);
    if (t.status == null) return `Не дошло: ${t.error ?? 'нет ответа'}`;
    return `Получатель ответил ${t.status}${body ? `: ${body}` : ''}`;
  };
  const fmtTime = (ts: number) =>
    new Date(ts * 1000).toLocaleString('ru', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });

  const createToken = () =>
    run(async () => {
      const t = await api.createApiToken(tokenName.trim() || 'Claude');
      created = { name: t.name, token: t.token };
      tokens = await api.apiTokens();
    });

  const deleteToken = (id: number) =>
    run(async () => {
      await api.deleteApiToken(id);
      tokens = tokens.filter((t) => t.id !== id);
    });

  async function copy(text: string, id: string) {
    await navigator.clipboard.writeText(text);
    copied = id;
    setTimeout(() => copied === id && (copied = ''), 1500);
  }

  const quote = (p: string) => (/\s/.test(p) ? `"${p}"` : p);
  const codeCmd = $derived(mcp?.binary && mcp.vault ? `claude mcp add bedrock -- ${quote(mcp.binary)} --vault ${quote(mcp.vault)}` : '');
  const desktopJson = $derived(
    mcp?.binary && mcp.vault
      ? JSON.stringify({ mcpServers: { bedrock: { command: mcp.binary, args: ['--vault', mcp.vault] } } }, null, 2)
      : '',
  );
  const remoteUrl = $derived(created && mcp?.server ? `${mcp.server}/mcp/${created.token}` : '');
  const curl = $derived(
    created && mcp?.server && status?.vaultId
      ? `curl -H "Authorization: Bearer ${created.token}" ${mcp.server}/api/vaults/${status.vaultId}/notes`
      : '',
  );
  const fmtDate = (ts: number) => new Date(ts * 1000).toLocaleDateString('ru', { day: 'numeric', month: 'short', year: 'numeric' });
</script>

{#snippet code(text: string, id: string)}
  <div class="code-row">
    <code>{text}</code>
    <button class="icon-btn" title={copied === id ? 'Скопировано' : 'Копировать'} aria-label="Копировать" onclick={() => copy(text, id)}>
      <Icon name={copied === id ? 'check' : 'copy'} size={14} />
    </button>
  </div>
{/snippet}

<svelte:window onkeydown={(e) => e.key === 'Escape' && onClose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay center" onclick={onClose}>
  <div class="modal wide" role="dialog" tabindex="-1" aria-label="Telegram и Claude" onclick={(e) => e.stopPropagation()}>
    <div class="modal-head">
      <div>
        <h2>Telegram и Claude</h2>
        <p>Напоминания в Telegram и доступ Claude к заметкам и задачам.</p>
      </div>
      <button class="icon-btn" aria-label="Закрыть" onclick={onClose}><Icon name="x" /></button>
    </div>

    <div class="modal-body">
      <section class="modal-section">
        <h3 class="section-head">
          <Icon name="send" />Telegram
          {#if tg?.linked}<span class="badge accent">подключён</span>{/if}
        </h3>
        {#if !connected}
          <p class="hint-text">Бот работает на сервере синхронизации. Подключите синхронизацию, и его можно будет привязать.</p>
          <button class="btn" style="align-self: flex-start;" onclick={onSync}>Настроить синхронизацию</button>
        {:else if !tg}
          <p class="hint-text">Проверяю сервер…</p>
        {:else}
          <p class="hint-text">
            Бот присылает напоминания (строки с ⏰, например «напомнить завтра в 9») и сообщает, когда участники меняют общие хранилища.
            Он работает на сервере, поэтому пишет, даже когда компьютер выключен.
          </p>

          <p class="code-label">Свой бот</p>
          {#if tg.own_bot}
            <div class="people">
              <div class="person">
                <span class="avatar" aria-hidden="true"><Icon name="send" size={12} /></span>
                <span class="who">@{tg.own_bot}</span>
                <span class="role">ваш бот</span>
                <button class="icon-btn" title="Убрать бота" aria-label="Убрать бота" disabled={busy} onclick={removeBot}><Icon name="x" size={14} /></button>
              </div>
            </div>
          {:else}
            <ol class="steps">
              <li>Откройте <button class="link-btn" onclick={() => openUrl('https://t.me/BotFather')}>@BotFather</button> и отправьте <code>/newbot</code>.</li>
              <li>Придумайте боту имя и адрес. BotFather пришлёт токен вида <code>123456789:AAE…</code></li>
              <li>Вставьте токен сюда:</li>
            </ol>
            <form
              class="inline-form"
              onsubmit={(e) => {
                e.preventDefault();
                if (botToken.trim()) setBot();
              }}
            >
              <input bind:value={botToken} placeholder="Токен от @BotFather" spellcheck="false" autocomplete="off" aria-label="Токен бота" />
              <button class="btn" disabled={busy || !botToken.trim()}>{busy ? 'Проверяю…' : 'Добавить бота'}</button>
            </form>
            {#if botError}<div class="notice error"><Icon name="warn" /><span>{botError}</span></div>{/if}
            {#if tg.server_bot}
              <p class="hint-text">Можно и без своего: на этом сервере есть общий бот @{tg.server_bot}.</p>
            {/if}
          {/if}

          {#if tg.available}
            {#if !tg.linked}
              <button class="btn primary" style="align-self: flex-start;" disabled={busy} onclick={linkTelegram}>Подключить чат с @{tg.bot}</button>
              {#if waiting}<p class="hint-text">Нажмите «Старт» в открывшемся чате с ботом, подключение появится здесь.</p>{/if}
            {:else}
              {#if tg.linked_via && tg.linked_via !== tg.bot}
                <div class="notice">
                  <Icon name="warn" />
                  <span>
                    Чат подключён через @{tg.linked_via}.
                    <button class="link-btn" disabled={busy} onclick={linkTelegram}>Переподключить через @{tg.bot}</button>
                  </span>
                </div>
              {:else}
                <p class="hint-text">Чат подключён через @{tg.linked_via}.</p>
              {/if}
              <label class="check-row">
                <input type="checkbox" checked={tg.notify_changes} disabled={busy} onchange={(e) => setChanges(e.currentTarget.checked)} />
                <span>Сообщать об изменениях в общих хранилищах<small>Раз в минуту тишины: кто и какие заметки изменил.</small></span>
              </label>
              <button class="btn" style="align-self: flex-start;" disabled={busy} onclick={unlink}>Отключить чат</button>
            {/if}
          {/if}
        {/if}
        {#if connected && e2e}
          <label class="check-row">
            <input type="checkbox" checked={status?.shareReminders} disabled={busy} onchange={(e) => setShare(e.currentTarget.checked)} />
            <span>
              Присылать напоминания этого хранилища
              <small>Оно зашифровано, поэтому приложение само передаёт серверу тексты строк с ⏰ и время. Остальные заметки остаются зашифрованными.</small>
            </span>
          </label>
        {:else if connected}
          <p class="hint-text">В серверном хранилище «{status?.vaultName}» сервер находит напоминания сам, даже когда приложение закрыто.</p>
        {/if}
      </section>

      <section class="modal-section">
        <h3 class="section-head">
          <Icon name="bell" />Вебхук напоминаний
          {#if hook?.configured}<span class="badge accent">включён</span>{/if}
        </h3>
        {#if !connected}
          <p class="hint-text">Нужна синхронизация с сервером.</p>
        {:else}
          <p class="hint-text">
            Сервер отправляет каждое напоминание из серверных хранилищ POST-запросом на ваш адрес, например своему агенту. Работает вместе с Telegram.
            Запрос подписан секретом: HMAC-SHA256 от «время.тело» в заголовке <code>X-Webhook-Signature-V2</code>.
          </p>
          <form
            class="stack-form"
            onsubmit={(e) => {
              e.preventDefault();
              if (hookDirty && hookUrl.trim()) saveHook();
            }}
          >
            <input bind:value={hookUrl} type="url" placeholder="https://…/webhook" spellcheck="false" autocomplete="off" aria-label="Адрес вебхука" />
            <input
              bind:value={hookSecret}
              type="password"
              placeholder={hook?.configured ? 'Секрет сохранён. Введите новый, только если меняете' : 'Секрет, от 32 символов'}
              autocomplete="new-password"
              aria-label="Секрет вебхука"
            />
            <div class="form-actions">
              <button class="btn" disabled={!!hookBusy || !hookDirty || !hookUrl.trim()}>{hookBusy === 'save' ? 'Сохраняю…' : 'Сохранить'}</button>
              <button
                class="btn"
                type="button"
                disabled={!!hookBusy || !hookUrl.trim() || (!hook?.configured && !hookSecret.trim())}
                onclick={testHook}>{hookBusy === 'test' ? 'Проверяю…' : 'Проверить'}</button
              >
              {#if hook?.configured}
                <button class="btn" type="button" disabled={!!hookBusy} onclick={removeHook}>Отключить</button>
              {/if}
            </div>
          </form>
          {#if hookError}<div class="notice error"><Icon name="warn" /><span>{hookError}</span></div>{/if}
          {#if hookTest}
            <div class="notice" class:error={!hookTest.delivered}>
              <Icon name={hookTest.delivered ? 'check' : 'warn'} /><span class="notice-text">{testText(hookTest)}</span>
            </div>
          {/if}
          {#if hook?.last_status}
            <p class="hint-text">
              Последняя доставка: {hook.last_status}{#if hook.last_delivery_at}, {fmtTime(hook.last_delivery_at)}{/if}
            </p>
          {/if}
          {#if e2e}
            <p class="hint-text">Хранилище «{status?.vaultName}» зашифровано: его напоминания приходят в Telegram, но не в вебхук.</p>
          {/if}
        {/if}
      </section>

      <section class="modal-section">
        <h3 class="section-head"><Icon name="plug" />Claude на этом компьютере</h3>
        <p class="hint-text">
          Claude Desktop и Claude Code смогут искать и читать заметки, добавлять задачи и напоминания и отмечать задачи выполненными.
          Работает с любым хранилищем, заметки не покидают компьютер.
        </p>
        {#if codeCmd}
          <p class="code-label">Claude Code</p>
          {@render code(codeCmd, 'code')}
          <p class="code-label">Claude Desktop: Settings → Developer → Edit Config</p>
          {@render code(desktopJson, 'desktop')}
        {:else if mcp}
          <p class="hint-text">Рядом с приложением нет программы bedrock-mcp. Соберите её: <code>cargo build --release -p bedrock-mcp</code></p>
        {/if}
      </section>

      <section class="modal-section">
        <h3 class="section-head"><Icon name="link" />Claude через сервер и API</h3>
        {#if !connected}
          <p class="hint-text">Нужна синхронизация с сервером.</p>
        {:else}
          <p class="hint-text">
            {#if e2e}Работает с серверными хранилищами аккаунта: это хранилище зашифровано, и сервер его не читает.
            {:else}Для claude.ai и скриптов: работает с серверными хранилищами аккаунта.{/if}
            Создайте токен и добавьте адрес как свой коннектор в claude.ai (Настройки → Коннекторы).
          </p>
          <form
            class="inline-form"
            onsubmit={(e) => {
              e.preventDefault();
              createToken();
            }}
          >
            <input bind:value={tokenName} placeholder="Название токена" aria-label="Название токена" />
            <button class="btn" disabled={busy}>Создать токен</button>
          </form>
          {#if created && remoteUrl}
            <div class="notice"><Icon name="lock" />Скопируйте сейчас: токен «{created.name}» больше не покажется. Он даёт доступ к вашим серверным хранилищам.</div>
            <p class="code-label">Адрес MCP-коннектора</p>
            {@render code(remoteUrl, 'remote')}
            {#if curl && !e2e}
              <p class="code-label">REST API</p>
              {@render code(curl, 'curl')}
            {/if}
          {/if}
          {#if tokens.length}
            <div class="people">
              {#each tokens as t (t.id)}
                <div class="person">
                  <span class="who">{t.name}</span>
                  <span class="role">с {fmtDate(t.created_at)}</span>
                  <button class="icon-btn" title="Отозвать" aria-label="Отозвать токен {t.name}" disabled={busy} onclick={() => deleteToken(t.id)}>
                    <Icon name="trash" size={14} />
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        {/if}
      </section>

      {#if error}<div class="notice error"><Icon name="warn" /><span>{error}</span></div>{/if}
    </div>
  </div>
</div>
