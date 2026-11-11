<script lang="ts">
  import { onMount } from 'svelte';

  type Asset = {
    id: string;
    recordId: string;
    categoryValue: string;
    editableFields: Record<string, string>;
    name: string;
    details: string;
    value: number;
    status: string;
    statusLabel: string;
    note?: string;
    valueHelp?: string;
    excludedFromTotal?: boolean;
  };

  type Category = { id: string; name: string; icon: string; assets: Asset[] };
  type AssetsPage = {
    client: { name: string; coClient: string; reportTitle: string };
    notice: string;
    funding: {
      percent: number;
      verified: number;
      leftToVerify: number;
      notFundable: number;
      notFundedPerClient: number;
      fundableTotal: number;
      footnote: string;
    };
    nextSteps: { title: string; description: string }[];
    categories: Category[];
    totalValue: number;
    totalNote: string;
    footer: { firm: string; address: string; phone: string; links: string[] };
  };

  let page = $state<AssetsPage | null>(null);
  let loading = $state(true);
  let loadError = $state('');
  let loginError = $state('');
  let authRequired = $state(false);
  let search = $state('');
  let collapsed = $state<Record<string, boolean>>({});
  let refreshing = $state(false);
  let showNewAsset = $state(false);
  let editingAsset = $state<Asset | null>(null);
  let mutationError = $state('');
  let mutationMessage = $state('');
  let savingAsset = $state(false);
  let newAssetForm = $state({
    category: '01 - Cash Account',
    description: '',
    reportAcctNo: '',
    assetType: 'Checking',
    value: '',
    comments: 'Client submitted through portal'
  });
  let editForm = $state<Record<string, string>>({});
  let originalEditForm = $state<Record<string, string>>({});

  const categoryOptions = $derived((() => {
    const options: string[] = [];
    for (const category of page?.categories ?? []) {
      for (const asset of category.assets) {
        if (asset.categoryValue && !options.includes(asset.categoryValue)) {
          options.push(asset.categoryValue);
        }
      }
    }
    return options;
  })());

  onMount(async () => {
    const callback = new URLSearchParams(window.location.hash.slice(1));
    const handoffCode = callback.get('actionstep_handoff');
    if (handoffCode || callback.has('actionstep_error')) {
      window.history.replaceState({}, '', `${window.location.pathname}${window.location.search}`);
      if (handoffCode) {
        try {
          const response = await fetch('/api/auth/complete', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ handoffCode })
          });
          if (!response.ok) throw new Error(`Sign-in could not be completed (${response.status}).`);
        } catch (error) {
          loginError = error instanceof Error ? error.message : 'Sign-in could not be completed.';
        }
      } else {
        loginError = 'Actionstep sign-in was not completed. Please try again.';
      }
    }

    try {
      await loadAssets();
    } catch (error) {
      loadError = error instanceof Error ? error.message : 'Unable to load assets.';
    } finally {
      loading = false;
    }
  });

  async function loadAssets() {
    const response = await fetch('/api/assets');
    if (response.status === 401) {
      authRequired = true;
      page = null;
      return;
    }
    if (!response.ok) {
      const result = await response.json().catch(() => null) as { error?: string } | null;
      throw new Error(result?.error ?? `Request failed (${response.status})`);
    }
    page = await response.json() as AssetsPage;
    authRequired = false;
    loginError = '';
  }

  async function refreshAssets() {
    refreshing = true;
    loadError = '';
    try {
      await loadAssets();
    } catch (error) {
      loadError = error instanceof Error ? error.message : 'Unable to load assets.';
    } finally {
      refreshing = false;
    }
  }

  function openEditAsset(asset: Asset) {
      mutationError = '';
      mutationMessage = '';
      editingAsset = asset;
      const values = {
        Asset_Value: asset.editableFields.Asset_Value ?? String(asset.value),
        Asset_Description: asset.editableFields.Asset_Description ?? '',
        ReportAcctNo: asset.editableFields.ReportAcctNo ?? '',
        Type: asset.editableFields.Type ?? '',
        Comments_for_Client: asset.editableFields.Comments_for_Client ?? '',
        LienAmount: asset.editableFields.LienAmount ?? '',
        Equity_Basis: asset.editableFields.Equity_Basis ?? ''
      };
      editForm = { ...values };
      originalEditForm = { ...values };
    }

  async function submitNewAsset(event: SubmitEvent) {
      event.preventDefault();
      await saveAsset('/api/assets', 'POST', newAssetForm);
      if (!mutationError) {
        showNewAsset = false;
        newAssetForm = {
          category: categoryOptions[0] ?? '01 - Cash Account',
          description: '',
          reportAcctNo: '',
          assetType: 'Checking',
          value: '',
          comments: 'Client submitted through portal'
        };
      }
    }

  async function submitAssetEdits(event: SubmitEvent) {
      event.preventDefault();
      if (!editingAsset) return;
      const fields: Record<string, string> = {};
      for (const field of Object.keys(editForm)) {
        if (editForm[field] !== originalEditForm[field]) fields[field] = editForm[field];
      }
      if (Object.keys(fields).length === 0) {
        mutationError = 'Change at least one field before saving.';
        return;
      }
      await saveAsset(
        `/api/assets/${encodeURIComponent(editingAsset.recordId)}`,
        'PUT',
        { fields }
      );
      if (!mutationError) editingAsset = null;
    }

  async function removeAsset(asset: Asset) {
      if (!window.confirm(`Mark “${asset.name}” as deleted in Actionstep?`)) return;
      mutationError = '';
      mutationMessage = '';
      savingAsset = true;
      try {
        const response = await fetch(`/api/assets/${encodeURIComponent(asset.recordId)}`, {
          method: 'DELETE'
        });
        if (!response.ok) throw await responseError(response);
        await loadAssets();
        mutationMessage = 'Asset marked as deleted in Actionstep.';
      } catch (error) {
        mutationError = error instanceof Error ? error.message : 'Unable to delete asset.';
      } finally {
        savingAsset = false;
      }
    }

  async function saveAsset(url: string, method: 'POST' | 'PUT', body: unknown) {
      mutationError = '';
      mutationMessage = '';
      savingAsset = true;
      try {
        const response = await fetch(url, {
          method,
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body)
        });
        if (!response.ok) throw await responseError(response);
        await loadAssets();
        mutationMessage = method === 'POST' ? 'Asset added to Actionstep.' : 'Asset updated in Actionstep.';
      } catch (error) {
        mutationError = error instanceof Error ? error.message : 'Unable to save asset.';
      } finally {
        savingAsset = false;
      }
    }

  async function responseError(response: Response) {
      const result = await response.json().catch(() => null) as { error?: string } | null;
      return new Error(result?.error ?? `Request failed (${response.status})`);
    }
  const visibleCategories = $derived(page?.categories
    .map((category) => ({
      ...category,
      assets: category.assets.filter((asset) =>
        `${asset.name} ${asset.details} ${asset.statusLabel}`.toLowerCase().includes(search.trim().toLowerCase())
      )
    }))
    .filter((category) => category.assets.length > 0 || (!search.trim() && category.assets.length === 0)) ?? []);

  function money(value: number) {
    return new Intl.NumberFormat('en-US', {
      style: 'currency', currency: 'USD', maximumFractionDigits: 0
    }).format(value);
  }

  function toggleCategory(id: string) {
    collapsed[id] = !collapsed[id];
  }
</script>

<svelte:head>
  <title>My Assets | InSight Law</title>
  <meta name="description" content="Review the assets currently listed in your estate plan." />
</svelte:head>

{#if loading}
  <div class="state-screen" role="status"><span class="loading-mark"></span><p>Loading your asset report</p></div>
{:else if loadError}
  <div class="state-screen error-screen" role="alert"><strong>We couldn't load your assets.</strong><p>{loadError}</p><button class="button button-primary" onclick={refreshAssets}>Try again</button></div>
{:else if authRequired}
  <div class="state-screen" role="status"><strong>Connect to Actionstep</strong><p>{loginError || 'Sign in to Actionstep to load your live asset report.'}</p><a class="button button-primary" href="/api/auth/login">Sign in to Actionstep</a></div>
{:else if page}
  <div class="portal">
    <header class="topbar">
      <a class="brand" href="/" aria-label="InSight Law home"><span class="brand-mark">I</span><span><strong>INSIGHT</strong><small>LAW · CLIENT PORTAL</small></span></a>
      <div class="topbar-actions">
        <button class="icon-button message-button" aria-label="Secure messages" title="Secure messages"><span aria-hidden="true">✉</span><span class="message-label">Messages</span></button>
        <span class="topbar-rule"></span>
        <button class="profile-button" aria-label="Client profile"><span class="avatar">{page.client.name.charAt(0)}</span><span class="profile-copy"><strong>{page.client.name}</strong><small>Client account</small></span><span class="profile-chevron">⌄</span></button>
      </div>
    </header>

    <aside class="sidebar" aria-label="Main navigation">
      <div class="nav-label">CLIENT PORTAL</div>
      <a class="nav-item" href="#home"><span class="nav-icon">⌂</span>Home</a>
      <a class="nav-item active" href="#assets" aria-current="page"><span class="nav-icon">▤</span>My Assets<span class="active-bar"></span></a>
      <a class="nav-item" href="#contacts"><span class="nav-icon">♧</span>My Contacts</a>
      <a class="nav-item" href="#documents"><span class="nav-icon">▧</span>My Documents</a>
      <a class="nav-item" href="#messages"><span class="nav-icon">✉</span>Messages</a>
      <a class="nav-item" href="#resources"><span class="nav-icon">▤</span>Resources</a>
      <div class="sidebar-bottom"><span class="sidebar-divider"></span><a class="nav-item sign-out" href="#sign-out"><span class="nav-icon">↪</span>Sign Out</a><div class="secure-side"><span class="secure-dot"></span>Secure client portal</div></div>
    </aside>

    <main id="assets" class="main-content">
      <div class="content-wrap">
        <div class="page-kicker">ESTATE PLAN <span>/</span> MY ASSETS</div>
        <div class="trust-reminder"><span class="trust-icon">✓</span><span>{page.notice}</span><span class="reminder-tag">ACTION NEEDED</span></div>

        <section class="overview" aria-label="Funding progress and next steps">
          <div class="funding-panel">
            <div class="panel-heading"><div><span class="eyebrow">YOUR ESTATE PLAN</span><h2>Funding progress</h2></div><button class="info-button" title="Funding progress excludes assets marked not fundable." aria-label="About funding progress">i</button></div>
            <div class="funding-content">
              <div class="gauge-wrap">
                <svg class="gauge" viewBox="0 0 320 178" role="img" aria-label={`${page.funding.percent}% of fundable assets verified`}>
                  <path class="gauge-track" d="M 34 148 A 126 126 0 0 1 286 148" pathLength="100"></path>
                  <path class="gauge-progress" d="M 34 148 A 126 126 0 0 1 286 148" pathLength="100" stroke-dasharray={`${page.funding.percent} 100`}></path>
                  <text class="gauge-value" x="160" y="92" text-anchor="middle">{page.funding.percent}%</text>
                  <text class="gauge-label" x="160" y="118" text-anchor="middle">funded</text>
                  <text class="gauge-caption" x="160" y="148" text-anchor="middle">{page.funding.verified} of {page.funding.fundableTotal} verified</text>
                </svg>
              </div>
              <div class="funding-stats">
                <div class="stat"><span class="stat-dot verified-dot"></span><span>Verified</span><strong>{page.funding.verified}</strong></div>
                <div class="stat"><span class="stat-dot review-dot"></span><span>Left to verify</span><strong>{page.funding.leftToVerify}</strong></div>
                <div class="stat"><span class="stat-dot neutral-dot"></span><span>Not fundable</span><strong>{page.funding.notFundable}</strong></div>
                <div class="stat"><span class="stat-dot attention-dot"></span><span>Not funded per client</span><strong>{page.funding.notFundedPerClient}</strong></div>
              </div>
            </div>
            <p class="panel-footnote"><span>i</span>{page.funding.footnote}</p>
          </div>

          <aside class="next-panel" aria-label="What to do next">
            <div class="panel-heading"><div><span class="eyebrow">YOUR CHECKLIST</span><h2>What to do next</h2></div><span class="step-count">{page.nextSteps.length} STEPS</span></div>
            <ol class="next-list">
              {#each page.nextSteps as step, index}
                <li><span class="step-number">0{index + 1}</span><span class="step-copy"><strong>{step.title}</strong><small>{step.description}</small></span><span class="step-arrow">›</span></li>
              {/each}
            </ol>
            <p class="next-note">You can edit asset details or add assets directly to this Actionstep matter.</p>
          </aside>
        </section>

        <section class="report-section" aria-label="Asset review report">
          <div class="report-heading">
            <div><span class="eyebrow">CLIENT ASSET INVENTORY</span><h1>{page.client.reportTitle}</h1><p>Live Actionstep data. Changes are saved to your matter.</p></div>
          </div>

          <div class="toolbar"><label class="search-box"><span aria-hidden="true">⌕</span><input aria-label="Search assets" placeholder="Search assets" bind:value={search} /></label><span class="asset-count">{visibleCategories.reduce((count, category) => count + category.assets.length, 0)} assets shown</span><div class="toolbar-actions"><button class="button button-outline" onclick={refreshAssets} disabled={refreshing || savingAsset}>{refreshing ? 'Refreshing…' : 'Refresh'}</button><button class="button button-primary add-button" onclick={() => { mutationError = ''; mutationMessage = ''; showNewAsset = true; }} disabled={savingAsset}><span aria-hidden="true">+</span>Add asset</button></div></div>
          {#if mutationError}<p class="mutation-message error-message" role="alert">{mutationError}</p>{/if}
          {#if mutationMessage}<p class="mutation-message success-message" role="status">{mutationMessage}</p>{/if}
          <div class="asset-table" role="table" aria-label="Assets list">
            <div class="table-header" role="row"><span role="columnheader">Asset description</span><span role="columnheader">Asset value</span><span role="columnheader">Funding status</span><span role="columnheader">Comments</span><span role="columnheader">Actions</span></div>
            {#each visibleCategories as category (category.id)}
              <div class="category-block">
                <button class="category-heading" aria-expanded={!collapsed[category.id]} onclick={() => toggleCategory(category.id)}>
                  <span class:collapsed={collapsed[category.id]} class="category-chevron" aria-hidden="true">⌄</span><span class="category-icon">{category.icon}</span><span class="category-title">{category.name}</span><span class="category-count">{category.assets.length}</span><span class="category-line"></span>
                </button>
                {#if !collapsed[category.id]}
                  {#if category.assets.length === 0}
                    <div class="empty-category">No {category.name.toLowerCase()} listed.</div>
                  {:else}
                    {#each category.assets as asset (asset.id)}
                      <article class="asset-row">
                        <div class="asset-description"><strong>{asset.name}</strong>{#if asset.details}<span>{asset.details}</span>{/if}{#if asset.valueHelp}<small class="asset-help">{asset.valueHelp}</small>{/if}</div>
                        <div class="asset-value"><span class="mobile-label">Asset value</span><span class="value-field">{money(asset.value)}</span>{#if asset.excludedFromTotal}<small class="excluded-note">* Excluded from total</small>{/if}</div>
                        <div class="asset-status"><span class="mobile-label">Funding status</span><span class={`status-pill ${asset.status}`}>{#if asset.status === 'verified'}✓{:else if asset.status === 'needs_verification'}!{:else if asset.status === 'undecided'}?{:else}−{/if} {asset.statusLabel}</span></div>
                        <div class="asset-comments"><span class="mobile-label">Comments</span>{#if asset.note}<p class="firm-note">{asset.note}</p>{:else}<span>—</span>{/if}</div>
                        <div class="asset-actions"><button class="text-button" onclick={() => openEditAsset(asset)} disabled={savingAsset}>Edit</button><button class="text-button delete-action" onclick={() => removeAsset(asset)} disabled={savingAsset}>Delete</button></div>
                      </article>
                    {/each}
                  {/if}
                {/if}
              </div>
            {/each}
            {#if visibleCategories.length === 0}<div class="no-results">No assets match “{search}”.</div>{/if}
          </div>

          <div class="bottom-actions"><div class="total-copy"><span>ESTIMATED TOTAL</span><strong>{money(page.totalValue)}</strong><small>{page.totalNote}</small></div></div>
        </section>
      </div>

      <footer class="page-footer"><div><strong>{page.footer.firm}</strong><span>{page.footer.address}</span><a href={`tel:${page.footer.phone}`}>{page.footer.phone}</a></div><nav aria-label="Legal links">{#each page.footer.links as link}<a href="#footer">{link}</a>{/each}</nav><span class="footer-secure"><span class="secure-dot"></span>Secure client portal</span></footer>
    </main>
  </div>

  {#if showNewAsset}
    <div class="modal-backdrop" role="presentation">
      <div class="asset-modal" role="dialog" aria-modal="true" aria-labelledby="new-asset-title" tabindex="-1">
        <div class="modal-heading"><div><span class="eyebrow">ACTIONSTEP ASSET</span><h2 id="new-asset-title">Add an asset</h2></div><button class="modal-close" aria-label="Close" onclick={() => showNewAsset = false}>×</button></div>
        <form onsubmit={submitNewAsset}>
          <label>Category<select bind:value={newAssetForm.category}>{#each (categoryOptions.length ? categoryOptions : ['01 - Cash Account']) as category}<option value={category}>{category}</option>{/each}</select></label>
          <label>Asset description<input bind:value={newAssetForm.description} required maxlength="2000" /></label>
          <label>Account number<input bind:value={newAssetForm.reportAcctNo} maxlength="2000" /></label>
          <label>Type<input bind:value={newAssetForm.assetType} required maxlength="2000" /></label>
          <label>Asset value<input type="text" inputmode="decimal" pattern="[0-9]+([.][0-9]+)?" bind:value={newAssetForm.value} required /></label>
          <label>Comments<textarea bind:value={newAssetForm.comments} maxlength="2000"></textarea></label>
          <p class="modal-note">This creates a new Actionstep record. Assets added here are marked as client-added for audit purposes.</p>
          {#if mutationError}<p class="mutation-message error-message" role="alert">{mutationError}</p>{/if}
          <div class="modal-actions"><button type="button" class="button button-quiet" onclick={() => showNewAsset = false} disabled={savingAsset}>Cancel</button><button type="submit" class="button button-primary" disabled={savingAsset}>{savingAsset ? 'Saving…' : 'Add asset'}</button></div>
        </form>
      </div>
    </div>
  {/if}

  {#if editingAsset}
    <div class="modal-backdrop" role="presentation">
      <div class="asset-modal edit-asset-modal" role="dialog" aria-modal="true" aria-labelledby="edit-asset-title" tabindex="-1">
        <div class="modal-heading"><div><span class="eyebrow">ACTIONSTEP ASSET</span><h2 id="edit-asset-title">Edit {editingAsset.name}</h2></div><button class="modal-close" aria-label="Close" onclick={() => editingAsset = null}>×</button></div>
        <form onsubmit={submitAssetEdits}>
          <label>Asset value<input type="text" inputmode="decimal" pattern="[0-9]+([.][0-9]+)?" bind:value={editForm.Asset_Value} required /></label>
          <label>Asset description<input bind:value={editForm.Asset_Description} maxlength="2000" /></label>
          <label>Account number<input bind:value={editForm.ReportAcctNo} maxlength="2000" /></label>
          <label>Type<input bind:value={editForm.Type} maxlength="2000" /></label>
          <label>Comments<textarea bind:value={editForm.Comments_for_Client} maxlength="2000"></textarea></label>
          <label>Lien amount<input type="text" inputmode="decimal" pattern="[0-9]*([.][0-9]+)?" bind:value={editForm.LienAmount} /></label>
          <label>Equity basis<input type="text" inputmode="decimal" pattern="[0-9]*([.][0-9]+)?" bind:value={editForm.Equity_Basis} /></label>
          <p class="modal-note">Only fields changed here are sent to Actionstep. The asset’s client-added marker is never changed.</p>
          {#if mutationError}<p class="mutation-message error-message" role="alert">{mutationError}</p>{/if}
          <div class="modal-actions"><button type="button" class="button button-quiet" onclick={() => editingAsset = null} disabled={savingAsset}>Cancel</button><button type="submit" class="button button-primary" disabled={savingAsset}>{savingAsset ? 'Saving…' : 'Save changes'}</button></div>
        </form>
      </div>
    </div>
  {/if}
{/if}
