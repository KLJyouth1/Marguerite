/**
 * MetadataCatalog — 部件档案目录（纯数据 + 纯函数，node 可测）。
 *
 * UI 不手写每个部件的说明 HTML：部件档案 `{id,name,english,category,intro,facts}`
 * 集中在这里，说明卡由模板根据 metadata 自动渲染。新增部件 = 追加一条档案，
 * 不碰模板。
 *
 * facts 是 [[k, v]] 键值对（语言中立原始值；标签由 UI i18n 处理或直接展示）。
 */

import type { PartMetadata, PartNode } from './PartNode'

export interface CatalogCategory {
  id: string
  label: string
}

/** 从 PartNode 树抽取所有档案，建 id → metadata 索引 */
export class MetadataCatalog {
  private readonly byId = new Map<string, PartMetadata>()
  readonly categories: CatalogCategory[]

  constructor(root: PartNode, categories: CatalogCategory[] = []) {
    this.categories = categories
    this.index(root)
  }

  private index(node: PartNode): void {
    this.byId.set(node.metadata.id, node.metadata)
    for (const child of node.children ?? []) this.index(child)
  }

  /** 取档案；未命中返回 null（说明卡应显示「无档案」而非编造） */
  get(id: string): PartMetadata | null {
    return this.byId.get(id) ?? null
  }

  has(id: string): boolean {
    return this.byId.has(id)
  }

  get size(): number {
    return this.byId.size
  }

  /** 按类别过滤部件 id（侧栏分组列表用） */
  idsByCategory(category: string): string[] {
    const out: string[] = []
    for (const [id, meta] of this.byId) {
      if (meta.category === category) out.push(id)
    }
    return out.sort()
  }

  /** 搜索（name / english / intro 子串，大小写不敏感） */
  search(query: string): PartMetadata[] {
    const q = query.trim().toLowerCase()
    if (!q) return []
    const out: PartMetadata[] = []
    for (const meta of this.byId.values()) {
      if (
        meta.name.toLowerCase().includes(q) ||
        meta.english.toLowerCase().includes(q) ||
        meta.intro.toLowerCase().includes(q)
      ) {
        out.push(meta)
      }
    }
    return out
  }
}
