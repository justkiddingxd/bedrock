<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import { api, errText, type Member, type RemoteVault, type Role, type SyncStatus, type VaultMode } from './api';
  import { SELF_HOSTED_DEFAULT, SHARED_SERVER } from './config';

  interface Props {
    status: SyncStatus | null;
    vaultName: string;
    onClose: () => void;
    onIntegrations: () => void;
  }
  let { status, vaultName, onClose, onIntegrations }: Props = $props();

  type Step = 'login' | 'vault' | 'status';
  let step = $state<Step>(untrack(() => (status?.configured && !status.needsLogin ? 'status' : 'login')));
  let preset = $state<'shared' | 'custom'>(SHARED_SERVER ? 'shared' : 'custom');
  let customServer = $state(localStorage.getItem('bedrock.server') ?? SELF_HOSTED_DEFAULT);
  let username = $state(localStorage.getItem('bedrock.username') ?? '');
  let password = $state('');
  let vaults = $state<RemoteVault[]>([]);
  let choice = $state('__new');
  let newName = $state('');
  let newMode = $state<VaultMode>('e2e');
  let vaultPassword = $state('');
  let vaultPassword2 = $state('');
  let members = $state<Member[]>([]);
  let invite = $state('');
  let inviteRole = $state<'editor' | 'viewer'>('editor');
  let busy = $state(false);
  let error = $state('');

  const server = $derived(preset === 'shared' ? SHARED_SERVER : customServer.trim());
  const isNew = $derived(choice === '__new');
  const chosen = $derived(vaults.find((v) => v.id === choice));
  const needsPassword = $derived(isNew ? newMode === 'e2e' : chosen?.mode !== 'server');
  const me = $derived(members.find((m) => m.username.toLowerCase() === status?.username?.toLowerCase()));
  const isOwner = $derived(me?.role === 'owner');

  const ROLE: Record<Role, string> = { owner: 'владелец', editor: 'редактор', viewer: 'читатель' };

  $effect(() => {
    if (status?.configured && step === 'login' && !status.needsLogin) step = 'status';
  });

  // Who else is in the connected vault.
  $effect(() => {
    if (step === 'status' && status?.configured && status.vaultId) {
      void status.vaultId;
      untrack(() => api.members().then((m) => (members = m)).catch(() => (members = [])));
    }
  });

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

  const login = (register: boolean) =>
    run(async () => {
      const res = await api.syncLogin(server, username.trim(), password, register);
      localStorage.setItem('bedrock.server', customServer);
      localStorage.setItem('bedrock.username', res.username);
      vaults = res.vaults;
      choice = vaults.find((v) => v.name === vaultName)?.id ?? (vaults.length ? vaults[0].id : '__new');
      newName = vaultName;
      password = '';
      step = 'vault';
    });

  const connect = () =>
    run(async () => {
      if (needsPassword) {
        if (isNew && vaultPassword !== vaultPassword2) throw new Error('Пароли хранилища не совпадают');
        if (vaultPassword.length < 8) throw new Error('Пароль хранилища — минимум 8 символов');
      }
      await api.syncConnect(isNew ? null : choice, isNew ? newName : null, isNew ? newMode : null, needsPassword ? vaultPassword : null);
      vaultPassword = vaultPassword2 = '';
      step = 'status';
    });

  const syncNow = () => run(async () => void (await api.syncNow()));
  const disconnect = () =>
    run(async () => {
      await api.syncDisconnect();
      members = [];
      step = 'login';
    });

  const addMember = () =>
    run(async () => {
      members = await api.addMember(invite.trim(), inviteRole);
      invite = '';
    });

  const removeMember = (name: string) =>
    run(async () => {
      members = await api.removeMember(name);
      // Leaving a shared vault: stop syncing it, the files stay here.
      if (name === me?.username) await disconnect();
    });

  function vaultMeta(v: RemoteVault) {
    const kind = v.mode === 'server' ? 'серверное' : 'зашифровано';
    return v.role === 'owner' ? kind : `${kind} · от ${v.owner}, ${ROLE[v.role]}`;
  }

  function fmtTime(ts: number | null) {
    if (!ts) return 'ещё не было';
    return new Date(ts * 1000).toLocaleString('ru', { dateStyle: 'medium', timeStyle: 'short' });
  }
  const stateText = $derived(
    status?.state === 'syncing' ? 'идёт синхронизация' : status?.state === 'error' ? (status.lastError ?? 'ошибка') : 'всё синхронизировано',
  );
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onClose()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="overlay center" onclick={onClose}>
  <div class="modal" class:wide={step === 'status'} role="dialog" tabindex="-1" aria-label="Синхронизация" onclick={(e) => e.stopPropagation()}>
    <div class="modal-head">
      <div>
        <h2>Синхронизация</h2>
        <p>
          {#if step === 'login'}Между устройствами и с другими людьми. Шифровать ли заметки, выбирается для каждого хранилища.
          {:else if step === 'vault'}Выберите хранилище на сервере для «{vaultName}».
          {:else}{status?.vaultName ?? vaultName} · {status?.username}{/if}
        </p>
      </div>
      <button class="icon-btn" aria-label="Закрыть" onclick={onClose}><Icon name="x" /></button>
    </div>

    {#if step === 'login'}
      <form
        class="modal-body"
        onsubmit={(e) => {
          e.preventDefault();
          login(false);
        }}
      >
        {#if status?.needsLogin}<div class="notice error"><Icon name="warn" />Сессия истекла — войдите снова.</div>{/if}
        {#if SHARED_SERVER}
          <div class="segmented">
            <button type="button" class:active={preset === 'shared'} onclick={() => (preset = 'shared')}>Общий сервер</button>
            <button type="button" class:active={preset === 'custom'} onclick={() => (preset = 'custom')}>Свой сервер</button>
          </div>
        {/if}
        {#if preset === 'custom'}
          <label class="field">Адрес сервера<input bind:value={customServer} placeholder="https://notes.example.com" spellcheck="false" /></label>
        {/if}
        <label class="field">Имя пользователя<input bind:value={username} autocomplete="username" spellcheck="false" /></label>
        <label class="field">Пароль<input type="password" bind:value={password} autocomplete="current-password" /></label>
        <div class="buttons">
          <button type="button" class="btn" disabled={busy || !username || !password} onclick={() => login(true)}>Создать аккаунт</button>
          <button type="submit" class="btn primary" disabled={busy || !username || !password}>{busy ? 'Вход…' : 'Войти'}</button>
        </div>
      </form>
    {:else if step === 'vault'}
      <form
        class="modal-body"
        onsubmit={(e) => {
          e.preventDefault();
          connect();
        }}
      >
        <div class="choices">
          {#each vaults as v (v.id)}
            <label class="choice"><input type="radio" bind:group={choice} value={v.id} />{v.name}<span class="meta">{vaultMeta(v)}</span></label>
          {/each}
          <label class="choice"><input type="radio" bind:group={choice} value="__new" />Новое хранилище</label>
        </div>
        {#if isNew}
          <label class="field">Название<input bind:value={newName} /></label>
          <div class="choices">
            <label class="choice tall">
              <input type="radio" bind:group={newMode} value="e2e" />
              <span><b>Зашифрованное</b><small>Сервер хранит только шифротекст, без текста и имён файлов. Нужен пароль хранилища.</small></span>
            </label>
            <label class="choice tall">
              <input type="radio" bind:group={newMode} value="server" />
              <span><b>Серверное</b><small>Сервер видит текст заметок: работают API, Claude через сервер и напоминания в Telegram без приложения.</small></span>
            </label>
          </div>
        {/if}
        {#if needsPassword}
          <label class="field">Пароль хранилища<input type="password" bind:value={vaultPassword} placeholder="минимум 8 символов" autocomplete="new-password" /></label>
          {#if isNew}
            <label class="field">Повторите пароль<input type="password" bind:value={vaultPassword2} autocomplete="new-password" /></label>
            <div class="notice"><Icon name="lock" />Пароль хранилища нельзя восстановить: без него данные на сервере не расшифровать. Он нужен на каждом устройстве и каждому участнику.</div>
          {/if}
        {/if}
        <div class="buttons">
          <button type="button" class="btn" onclick={() => (step = 'login')}>Назад</button>
          <button type="submit" class="btn primary" disabled={busy || (needsPassword && !vaultPassword)}>
            {busy ? (needsPassword ? 'Проверяю ключ…' : 'Подключаю…') : 'Подключить'}
          </button>
        </div>
      </form>
    {:else}
      <div class="modal-body">
        <dl class="kv">
          <dt>Состояние</dt>
          <dd><span class="dot {status?.state}"></span>{stateText}</dd>
          <dt>Последний раз</dt>
          <dd>{fmtTime(status?.lastSync ?? null)}</dd>
          <dt>Сервер</dt>
          <dd>{status?.server ?? '—'}</dd>
          <dt>Хранилище</dt>
          <dd>
            {status?.mode === 'server' ? 'Серверное: сервер видит текст' : 'Зашифровано на устройстве (XChaCha20-Poly1305)'}
          </dd>
          {#if status?.readOnly}
            <dt>Доступ</dt>
            <dd>только чтение: ваши правки остаются на этом устройстве</dd>
          {/if}
        </dl>

        <section class="modal-section">
          <h3 class="section-head"><Icon name="people" />Участники</h3>
          {#if members.length}
            <div class="people">
              {#each members as m (m.username)}
                <div class="person">
                  <span class="avatar" aria-hidden="true">{m.username.charAt(0)}</span>
                  <span class="who">{m.username}{m.username === me?.username ? ' (вы)' : ''}</span>
                  <span class="role">{ROLE[m.role]}</span>
                  {#if m.role !== 'owner' && (isOwner || m.username === me?.username)}
                    <button
                      class="icon-btn"
                      title={m.username === me?.username ? 'Выйти из хранилища' : 'Убрать доступ'}
                      aria-label={m.username === me?.username ? 'Выйти из хранилища' : `Убрать доступ у ${m.username}`}
                      disabled={busy}
                      onclick={() => removeMember(m.username)}><Icon name="x" size={14} /></button
                    >
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          {#if isOwner}
            <form
              class="inline-form"
              onsubmit={(e) => {
                e.preventDefault();
                if (invite.trim()) addMember();
              }}
            >
              <input bind:value={invite} placeholder="Имя пользователя на этом сервере" spellcheck="false" aria-label="Кого пригласить" />
              <div class="segmented" role="group" aria-label="Права">
                <button type="button" class:active={inviteRole === 'editor'} onclick={() => (inviteRole = 'editor')}>Редактор</button>
                <button type="button" class:active={inviteRole === 'viewer'} onclick={() => (inviteRole = 'viewer')}>Читатель</button>
              </div>
              <button class="btn" disabled={busy || !invite.trim()}>Пригласить</button>
            </form>
            {#if status?.mode === 'e2e'}
              <p class="hint-text">Хранилище зашифровано: сообщите участнику пароль хранилища, сервер его не знает.</p>
            {:else}
              <p class="hint-text">Правки участников сводятся автоматически, как между вашими устройствами.</p>
            {/if}
          {/if}
        </section>

        <button class="link-row" onclick={onIntegrations}><Icon name="plug" size={15} />Telegram, API и Claude</button>

        {#if status?.lastReport && (status.lastReport.conflicts.length || status.lastReport.errors.length)}
          <div class="notice">
            <Icon name="warn" />
            <span>
              {#each status.lastReport.conflicts as c (c)}<div>Конфликт: сохранена копия «{c}»</div>{/each}
              {#each status.lastReport.errors as e (e)}<div>{e}</div>{/each}
            </span>
          </div>
        {/if}
        <div class="buttons">
          <button class="btn danger" disabled={busy} onclick={disconnect}>Отключить</button>
          <button class="btn primary" disabled={busy || status?.state === 'syncing'} onclick={syncNow}>Синхронизировать</button>
        </div>
      </div>
    {/if}

    {#if error}<div style="padding: 0 24px 20px;"><div class="notice error"><Icon name="warn" /><span>{error}</span></div></div>{/if}
  </div>
</div>
