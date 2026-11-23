// 图标路径（与原型一致），24×24 描边
export const P: Record<string, string> = {
 play:'<path d="M7 4.5v15l13-7.5z" fill="currentColor" stroke="none"/>',
 pause:'<path d="M6.5 4.5h4v15h-4zM13.5 4.5h4v15h-4z" fill="currentColor" stroke="none"/>',
 next:'<path d="M5 5v14l10-7zM17 5h2.5v14H17z" fill="currentColor" stroke="none"/>',
 prev:'<path d="M19 5v14L9 12zM4.5 5H7v14H4.5z" fill="currentColor" stroke="none"/>',
 shuffle:'<path d="M3 7h3.5c5 0 6 10 11 10H21M3 17h3.5c1.6 0 2.8-1 3.8-2.4M21 7h-3.5c-1.6 0-2.8 1-3.8 2.4M18 4l3 3-3 3M18 14l3 3-3 3"/>',
 repeat:'<path d="M4 11V9a3 3 0 0 1 3-3h12M16 3l3 3-3 3M20 13v2a3 3 0 0 1-3 3H5M8 21l-3-3 3-3"/>',
 vol:'<path d="M4 9.5v5h3.5L12 18V6L7.5 9.5z" fill="currentColor" stroke="none"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
 scan:'<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5M11 7.5V11l2.2 1.6"/>',
 oled:'<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M9 20h6M12 16.5V20M8 10.5l2.5-2.5M11.5 12.5l4-4"/>',
 eq:'<path d="M5 4v6M5 14v6M12 4v2M12 10v10M19 4v9M19 17v3"/><circle cx="5" cy="12" r="2"/><circle cx="12" cy="8" r="2"/><circle cx="19" cy="15" r="2"/>',
 expand:'<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
 mini:'<rect x="3" y="4" width="18" height="16" rx="2.5"/><rect x="12" y="12" width="6.5" height="5" rx="1" fill="currentColor"/>',
 music:'<path d="M9 18V6l11-2.5V16"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>',
 film:'<rect x="3.5" y="3.5" width="17" height="17" rx="2"/><path d="M7.5 3.5v17M16.5 3.5v17M3.5 8h4M3.5 12h4M3.5 16h4M16.5 8h4M16.5 12h4M16.5 16h4"/>',
 video:'<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M10 9.5v5l4.5-2.5z" fill="currentColor"/>',
 book:'<path d="M5 4.5h10.5a3 3 0 0 1 3 3V20H8a3 3 0 0 1-3-3zM5 17a3 3 0 0 1 3-3h10.5"/>',
 anime:'<path d="M12 3l2.2 5.6L20 9.3l-4.4 3.9 1.3 5.8-4.9-3-4.9 3 1.3-5.8L4 9.3l5.8-.7z"/>',
 convert:'<path d="M4 8h13l-3.5-3.5M20 16H7l3.5 3.5"/>',
 gear:'<circle cx="12" cy="12" r="3"/><path d="M12 3v2.5M12 18.5V21M3 12h2.5M18.5 12H21M5.6 5.6l1.8 1.8M16.6 16.6l1.8 1.8M18.4 5.6l-1.8 1.8M7.4 16.6l-1.8 1.8"/>',
 search:'<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
 heart:'<path d="M12 20s-7.5-4.6-7.5-10A4.3 4.3 0 0 1 12 7.2 4.3 4.3 0 0 1 19.5 10c0 5.4-7.5 10-7.5 10z"/>',
 list:'<path d="M4 6.5h16M4 12h16M4 17.5h10"/>',
 close:'<path d="M6 6l12 12M18 6L6 18"/>',
 folder:'<path d="M3.5 7a2 2 0 0 1 2-2H10l2 2.5h6.5a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
 home:'<path d="M4 11l8-7 8 7v8a1.5 1.5 0 0 1-1.5 1.5H14v-6h-4v6H5.5A1.5 1.5 0 0 1 4 19z"/>'
};

// 原型之外补充的图标
Object.assign(P, {
  plus: '<path d="M12 5v14M5 12h14"/>',
  trash: '<path d="M5 7h14M10 7V4.5h4V7M7 7l1 13h8l1-13"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/>',
  more: '<circle cx="5" cy="12" r="1.4" fill="currentColor"/><circle cx="12" cy="12" r="1.4" fill="currentColor"/><circle cx="19" cy="12" r="1.4" fill="currentColor"/>',
  back: '<path d="M15 5l-7 7 7 7"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  sub: '<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="M7 13h4M13 13h4M7 16h10"/>',
  star: '<path d="M12 3l2.6 5.6 6.1.7-4.5 4.2 1.2 6L12 16.6 6.6 19.5l1.2-6L3.3 9.3l6.1-.7z"/>',
  download: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
  undo: '<path d="M9 7L4 12l5 5M4 12h11a5 5 0 0 1 0 10h-3"/>',
  image: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><circle cx="9" cy="10" r="2"/><path d="M21 16l-5-5-9 9"/>',
});
