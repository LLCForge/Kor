// [g26] Oturum ağacı: her kayıt isteğe bağlı parentId taşır. Pi'deki JSONL yaprak işaretçisi değil; dosya kimliğiyle bağ.

export interface SessionTreeRow {
  id: string
  title: string
  parentId?: string
}

export interface SessionTreeNode extends SessionTreeRow {
  children: SessionTreeNode[]
}

export function buildSessionTree(rows: readonly SessionTreeRow[]): SessionTreeNode[] {
  const nodes = new Map<string, SessionTreeNode>()
  for (const row of rows) nodes.set(row.id, { ...row, children: [] })
  const roots: SessionTreeNode[] = []
  for (const node of nodes.values()) {
    const parent = node.parentId ? nodes.get(node.parentId) : undefined
    if (parent && parent.id !== node.id) parent.children.push(node)
    else roots.push(node)
  }
  const byTitle = (a: SessionTreeNode, b: SessionTreeNode) => a.title.localeCompare(b.title, 'tr')
  const sortDeep = (list: SessionTreeNode[]) => {
    list.sort(byTitle)
    for (const node of list) sortDeep(node.children)
  }
  sortDeep(roots)
  return roots
}

export function formatSessionTree(nodes: readonly SessionTreeNode[], indent = 0): string {
  const lines: string[] = []
  for (const node of nodes) {
    lines.push(`${'  '.repeat(indent)}- ${node.title} [${node.id}]`)
    lines.push(formatSessionTree(node.children, indent + 1))
  }
  return lines.filter(Boolean).join('\n')
}
