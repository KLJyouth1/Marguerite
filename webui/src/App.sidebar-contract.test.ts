import { describe, expect, it } from 'vitest'

import appSourceRaw from './App.vue?raw'

// The workspace stores .vue sources with CRLF line endings; normalize so the
// source-contract assertions can use LF-only substring expectations.
const appSource = appSourceRaw.replace(/\r\n/g, '\n')

describe('App sidebar chrome contract', () => {
  it('renders the Marguerite brand as a non-interactive lockup', () => {
    const brandStart = appSource.indexOf('<!-- Brand -->')
    const brandEnd = appSource.indexOf('<button', brandStart)
    const brandMarkup = appSource.slice(brandStart, brandEnd)

    expect(brandMarkup).toContain('<div class="sidebar-brand-lockup">')
    expect(brandMarkup).not.toContain('<router-link')
    expect(brandMarkup).not.toContain('@click')
    expect(brandMarkup).not.toContain('to="/overview"')
  })

  it('keeps project selection out of the primary sidebar controls', () => {
    const newTaskStart = appSource.indexOf('class="sidebar-new-session"')
    const workNavStart = appSource.indexOf('<router-link', newTaskStart)
    const primaryControls = appSource.slice(newTaskStart, workNavStart)

    expect(primaryControls).not.toContain('@click="openProjectPicker"')
    expect(primaryControls).not.toContain("t('workspaces.chooseProject')")
  })

  it('clears app-wide approvals for local and cross-view session deletion', () => {
    const crossViewStart = appSource.indexOf('function handleLocalSessionsDeleted')
    const crossViewEnd = appSource.indexOf('async function deleteSessions', crossViewStart)
    const crossViewDelete = appSource.slice(crossViewStart, crossViewEnd)
    expect(crossViewDelete).toContain('appStore.removePendingApprovalsForSessions(deleted)')

    const bulkStart = appSource.indexOf('async function onBulkDeleteSessions')
    const bulkEnd = appSource.indexOf('async function onDeleteSession', bulkStart)
    const bulkDelete = appSource.slice(bulkStart, bulkEnd)
    expect(bulkDelete).toContain('appStore.removePendingApprovalsForSessions(deleted)')

    const singleStart = appSource.indexOf('async function onDeleteSession')
    const singleEnd = appSource.indexOf('// Topbar approval pill', singleStart)
    const singleDelete = appSource.slice(singleStart, singleEnd)
    expect(singleDelete).toContain('appStore.removePendingApprovalsForSessions(deleted)')
  })

  it('bounds automatic sidebar RPCs after chat bootstrap admission', () => {
    expect(appSource).toContain('useSessions(\n  optionalSessionRpcCallOptions,\n)')
    expect(appSource).toContain('useAgentOptions(optionalSessionRpcCallOptions)')
    expect(appSource).toContain(
      'useSessionListSubscription({\n  rpc: rpcStore,\n  callOptions: optionalSessionRpcCallOptions,',
    )
  })
})
