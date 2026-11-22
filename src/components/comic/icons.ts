// 阅读器里翻页方式等按钮用的小图标（24×24 描边，配合 <svg viewBox="0 0 24 24"> 使用）
export const FLOW_ICON: Record<string, string> = {
  // 左右翻页：两页并排 + 左右箭头
  h: '<rect x="4" y="5" width="7" height="12" rx="1"/><rect x="13" y="5" width="7" height="12" rx="1"/><path d="M8 20.5h8M8 20.5l1.6-1.4M16 20.5l-1.6-1.4"/>',
  // 上下翻页：两页上下 + 上下箭头
  v: '<rect x="5" y="3" width="11" height="8" rx="1"/><rect x="5" y="13" width="11" height="8" rx="1"/><path d="M20 8v8M20 8l-1.4 1.6M20 16l-1.4-1.6"/>',
  // 上下连续滚动：长条
  vscroll: '<path d="M7 2v20M17 2v20"/><path d="M7 8h10M7 15h10" stroke-dasharray="2 2"/><path d="M12 10.5v3M12 13.5l-1.2-1.2M12 13.5l1.2-1.2"/>',
  // 左右连续滚动：横条
  hscroll: '<path d="M2 7h20M2 17h20"/><path d="M8 7v10M16 7v10" stroke-dasharray="2 2"/><path d="M10.5 12h3M13.5 12l-1.2-1.2M13.5 12l-1.2 1.2"/>',
  // 自动滚动
  auto: '<rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 9l3 3 3-3M9 13l3 3 3-3"/>',
  page: '<path d="M3 5.5c3-1 6-1 9 1 3-2 6-2 9-1v13c-3-1-6-1-9 1-3-2-6-2-9-1z"/><path d="M12 6.5v13"/>',
  scroll: '<rect x="6" y="2.5" width="12" height="19" rx="2"/><path d="M9 7h6M9 10.5h6M9 14h6M9 17.5h4"/>',
};
