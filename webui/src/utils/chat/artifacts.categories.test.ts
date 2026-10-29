import { describe, expect, it } from 'vitest'
import {
  artifactCategory,
  artifactCategoryLabel,
  artifactIconName,
  artifactKindPill,
  canPreview,
} from './artifacts'

describe('spreadsheet artifacts', () => {
  it('classifies spreadsheet extensions as sheet', () => {
    for (const extension of ['xlsx', 'xls', 'xlsm', 'ods', 'csv', 'tsv']) {
      const artifact = { name: `book.${extension}`, mime: 'application/octet-stream' }
      expect(artifactCategory(artifact)).toBe('sheet')
      expect(artifactIconName(artifact)).toBe('fileSpreadsheet')
    }
  })

  it('classifies spreadsheet MIME types even without a known extension', () => {
    const mimes = [
      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
      'application/vnd.ms-excel',
      'application/vnd.oasis.opendocument.spreadsheet',
      'text/csv',
      'text/tab-separated-values',
    ]
    for (const mime of mimes) {
      expect(artifactCategory({ name: 'export.bin', mime })).toBe('sheet')
    }
  })

  it('keeps spreadsheets download-only and labelled sheet', () => {
    const artifact = { name: 'pricing.csv', mime: 'text/csv', size: 12288 }
    expect(canPreview(artifact)).toBe(false)
    expect(artifactCategoryLabel(artifact)).toBe('sheet')
    expect(artifactKindPill(artifact)).toBe('CSV')
  })
})

describe('archive artifacts', () => {
  it('classifies archive extensions as archive', () => {
    for (const extension of ['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'rar']) {
      const artifact = { name: `bundle.${extension}`, mime: 'application/octet-stream' }
      expect(artifactCategory(artifact)).toBe('archive')
      expect(artifactIconName(artifact)).toBe('archive')
      expect(canPreview(artifact)).toBe(false)
    }
  })

  it('classifies archive MIME types', () => {
    expect(artifactCategory({ name: 'bundle.bin', mime: 'application/zip' })).toBe('archive')
    expect(artifactCategory({ name: 'bundle.bin', mime: 'application/gzip' })).toBe('archive')
    expect(artifactCategoryLabel({ name: 'bundle.zip' })).toBe('archive')
  })
})

describe('3D model artifacts', () => {
  it('classifies model extensions as model3d', () => {
    for (const extension of ['glb', 'gltf', 'obj', 'stl', 'fbx', 'step', 'stp', 'dwg']) {
      const artifact = { name: `part.${extension}`, mime: 'application/octet-stream' }
      expect(artifactCategory(artifact)).toBe('model3d')
      expect(artifactIconName(artifact)).toBe('boxes')
      expect(canPreview(artifact)).toBe(false)
    }
  })

  it('labels model3d as 3D', () => {
    expect(artifactCategoryLabel({ name: 'part.glb' })).toBe('3D')
    expect(artifactKindPill({ name: 'part.glb' })).toBe('GLB')
  })
})

describe('classification precedence for the new categories', () => {
  it('lets an explicit MIME type win over the extension', () => {
    expect(artifactCategory({ name: 'report.csv', mime: 'text/plain' })).toBe('document')
    expect(artifactCategory({ name: 'report.csv', mime: 'application/pdf' })).toBe('document')
  })

  it('falls back to the extension for generic MIME types', () => {
    expect(artifactCategory({ name: 'report.xlsx' })).toBe('sheet')
    expect(artifactCategory({ name: 'report.xlsx', mime: 'application/octet-stream' })).toBe('sheet')
  })

  it('leaves types without a previewer in the plain file bucket', () => {
    for (const name of ['brief.docx', 'slides.pptx', 'poster.psd', 'scene.blend']) {
      const artifact = { name }
      expect(artifactCategory(artifact)).toBe('file')
      expect(artifactIconName(artifact)).toBe('fileText')
      expect(canPreview(artifact)).toBe(false)
    }
  })
})
