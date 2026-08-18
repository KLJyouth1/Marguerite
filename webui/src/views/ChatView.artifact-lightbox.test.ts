import { describe, expect, it } from 'vitest'
import chatViewSource from './ChatView.vue?raw'
import controllerSource from '../composables/chat/useChatArtifactShareController.ts?raw'

// The artifact preview routing used to live inline in ChatView.vue and was
// guarded here by slicing the component source. The orchestration layer moved
// into `useChatArtifactShareController`, so the same invariant is now asserted
// against the controller — plus a wiring check so the routing cannot silently
// become dead code if ChatView ever stops instantiating the controller.
describe('ChatView artifact preview routing', () => {
  it('routes visual artifacts to the lightbox before inline or unsupported fallbacks', () => {
    const start = controllerSource.indexOf('function openArtifact(')
    const end = controllerSource.indexOf('\n  function closeDeliverables', start)
    const openArtifactSource = controllerSource.slice(start, end)

    expect(start).toBeGreaterThan(-1)
    expect(end).toBeGreaterThan(start)
    expect(openArtifactSource.indexOf("artifactCategory(artifact) === 'visual'"))
      .toBeGreaterThan(-1)
    expect(openArtifactSource.indexOf("artifactCategory(artifact) === 'visual'"))
      .toBeLessThan(openArtifactSource.indexOf('isInlineMediaArtifact(artifact)'))
    expect(openArtifactSource).toContain('artifactImageLightbox.open({')
  })

  it('keeps ChatView wired to the artifact/share controller', () => {
    expect(chatViewSource).toContain('useChatArtifactShareController(')
    expect(chatViewSource).toContain('openArtifact,')
  })
})
