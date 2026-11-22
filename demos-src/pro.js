/* ===== 升级版 demo 的专属逻辑：音乐库分组搜索、番剧详情、漫画与 AI 翻译、转换回写、AI 字幕、AI 服务设置 =====
   全部是界面演示：不读写任何文件，不调用任何网络接口，AI 结果是预先写好的示例。 */
(function(){
'use strict';
const {$,$$,icon,toast,coverCSS,fmtT}=MP,sleep=ms=>new Promise(r=>setTimeout(r,ms));
const esc=s=>String(s).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));

/* ---------- 0. 扩充曲库（新增曲目为纯音乐） ---------- */
[['潮汐档案 · 序','林间信号','潮汐档案','FLAC 24/96',72,205,57,[0,3,5,4],72],['北站','北纬三十度','站台以北','MP3 320k',80,262,52,[0,5,3,4],100],['晒被子的下午','白昼梦游者','夏日备忘录','ALAC',76,34,60,[0,4,3,5],88],
 ['电子羊','Synthetic Tide','电子羊之梦','WAV',84,315,50,[0,2,5,4],122],['松风','青鸾','听松集','APE',78,146,55,[0,3,0,4],66],['待机画面','Null Meridian','Boot Sequence','Opus 160k',70,180,48,[0,5,2,4],128],
 ['巷口','巷口录音室','胶片与雨','DSD64',82,46,58,[0,3,5,4],78],['彗星轨道','星屑邮局','深空来电','OGG Vorbis',88,228,53,[0,4,2,5],86],['防波堤','林间信号','潮汐档案','FLAC 24/96',74,196,57,[0,5,4,3],80],
 ['午夜便利店','Synthetic Tide','电子羊之梦','WAV',90,326,50,[0,5,3,2],116]
].forEach(x=>MP.songs.push({t:x[0],a:x[1],al:x[2],fmt:x[3],dur:x[4],hue:x[5],root:x[6],prog:x[7],bpm:x[8],ly:[],lrc:[],id:MP.songs.length}));
const PL=[128,96,74,61,12,88,45,33,3,20,57,0,9,41,7,66,2,0],AD=[420,300,260,5,180,9,150,90,3,330,12,2,200,60,8,1,240,6],LAST=[1,2,3,5,160,1,20,45,200,30,6,999,120,14,95,2,300,999];
const FAV=new Set([0,1,5,10,15]);
MP.songs.forEach((s,i)=>{s.plays=PL[i]??0;s.added=AD[i]??30;s.last=LAST[i]??30});

/* ---------- 1. 正在播放：唱片外圈频谱、播放队列 ---------- */
MP.spin($('#vinyl'),9);
MP.canvas($('#ringc'),(g,w,h,t)=>{g.clearRect(0,0,w,h);if(document.body.dataset.view!=='now')return;
  const n=72,s=MP.spectrum(n/2),cx=w/2,cy=h/2,R=w*.335,hue=MP.cur().hue;
  for(let i=0;i<n;i++){const v=s[i<n/2?i:n-1-i],a=i/n*Math.PI*2-Math.PI/2,len=2+v*w*.1;
    g.strokeStyle=`hsl(${hue+(i<n/2?i:n-i)*2.2} 90% 72%/${.18+v*.6})`;g.lineWidth=3;g.lineCap='round';g.beginPath();
    g.moveTo(cx+Math.cos(a)*R,cy+Math.sin(a)*R);g.lineTo(cx+Math.cos(a)*(R+len),cy+Math.sin(a)*(R+len));g.stroke()}});
$('#qbtn').onclick=e=>{const on=$('#stage').classList.toggle('q');e.currentTarget.classList.toggle('on',on);MP.setView('now');MP.state.ly=-2};

/* ---------- 2. 音乐库：搜索、智能分组、快捷播放 ---------- */
const LOSSLESS=/FLAC|ALAC|APE|WAV|DSD/;
const SMART=[
 ['hot','常听','播放 30 次以上',s=>s.plays>=30,330],['new','最近添加','近两周入库',s=>s.added<=14,200],
 ['fav','我的收藏','标记为喜欢',s=>FAV.has(s.id),350],['hifi','无损音质','FLAC / APE / DSD 等',s=>LOSSLESS.test(s.fmt),160],['cold','很久没听','超过三个月没播放',s=>s.last>=90,40]];
const M={f:'all',g:'none',q:'',sort:'def'};
/* 用户自建歌单（保存在浏览器本地） */
let PLS=MP.store.get('pls',null)||[{id:'p1',name:'通勤路上',ids:[1,3,7,15]},{id:'p2',name:'写代码用',ids:[5,13,9]}];
const savePl=()=>MP.store.set('pls',PLS),curPl=()=>M.f.startsWith('pl:')?PLS.find(p=>p.id===M.f.slice(3)):null;
function askName(title,val,cb){const m=MP.modal('mp-pl',`<h3>${title}<button class="mp-x">✕</button></h3><p class="mp-desc">给歌单起个名字。</p><input id="pl-n" class="tin" style="width:100%" maxlength="30" value="${esc(val||'')}" placeholder="例如：深夜循环"><div style="margin-top:14px;display:flex;gap:10px"><button class="mp-btn" id="pl-ok">确定</button><button class="mp-btn ghost mp-x">取消</button></div>`);
  const i=$('#pl-n',m),ok=()=>{const v=i.value.trim();if(!v)return toast('名字不能为空');m.classList.remove('open');cb(v)};$('#pl-ok',m).onclick=ok;i.onkeydown=e=>{if(e.key==='Enter')ok()};setTimeout(()=>{i.focus();i.select()},80)}
function newPl(songId){askName('新建歌单','',name=>{const p={id:'p'+Date.now(),name,ids:songId!=null?[songId]:[]};PLS.push(p);savePl();M.f='pl:'+p.id;MP.setView('music');mRender();toast(songId!=null?`已新建「${name}」并加入 1 首`:`已新建歌单「${name}」，在歌曲行上点“加入歌单”来添加`)})}
const plm=$('#plmenu');
function plMenu(btn,id){const r=btn.getBoundingClientRect();plm.innerHTML='<h6>加入歌单</h6>'+PLS.map(p=>`<button data-pid="${p.id}">${esc(p.name)}<i>${p.ids.includes(id)?'✓':''}</i></button>`).join('')+'<button data-pid="new">＋ 新建歌单…</button>';
  plm.classList.add('open');plm.style.left=Math.max(8,Math.min(innerWidth-210,r.right-190))+'px';plm.style.top=Math.min(innerHeight-plm.offsetHeight-8,r.bottom+4)+'px';
  plm.onclick=e=>{const b=e.target.closest('[data-pid]');if(!b)return;plm.classList.remove('open');if(b.dataset.pid==='new')return newPl(id);const p=PLS.find(x=>x.id===b.dataset.pid),k=p.ids.indexOf(id);
    if(k>=0){p.ids.splice(k,1);toast(`已从「${p.name}」移除`)}else{p.ids.push(id);toast(`已加入「${p.name}」`)}savePl();mRender()}}
document.addEventListener('pointerdown',e=>{if(!e.target.closest('#plmenu,[data-pladd]'))plm.classList.remove('open')});
$('#msort').insertAdjacentHTML('beforebegin','<select class="sel" id="mgrp"><option value="none">不分组</option><option value="a">按艺人分组</option><option value="al">按专辑分组</option><option value="fmt">按格式分组</option></select>');
function mFiltered(){
  const sm=SMART.find(x=>x[0]===M.f),pl=curPl();const hitq=s=>!M.q||(s.t+s.a+s.al+s.fmt).toLowerCase().includes(M.q);
  if(pl)return pl.ids.map(i=>MP.songs[i]).filter(s=>s&&hitq(s));
  let l=MP.songs.filter(s=>!s.trial&&s.trial!==0&&(!sm||sm[3](s))&&hitq(s));
  if(M.sort==='plays')l.sort((a,b)=>b.plays-a.plays);else if(M.sort==='added')l.sort((a,b)=>a.added-b.added);else if(M.sort==='title')l.sort((a,b)=>a.t.localeCompare(b.t,'zh'));
  else if(M.f==='hot')l.sort((a,b)=>b.plays-a.plays);else if(M.f==='new')l.sort((a,b)=>a.added-b.added);
  return l;
}
function mRender(){
  $('#smart').innerHTML=SMART.map(x=>{const n=MP.songs.filter(s=>!s.trial&&s.trial!==0&&x[3](s)).length;return `<div class="sc${M.f===x[0]?' on':''}" data-f="${x[0]}" style="background-image:${coverCSS(x[4],1)}"><b>${x[1]}</b><span>${n} 首 · ${x[2]}</span><button data-play="${x[0]}" title="立即播放">${icon('play')}</button></div>`}).join('');
  $('#mgroup').innerHTML=[['all','全部歌曲']].concat(SMART).map(x=>`<button class="mp-chip${M.f===x[0]?' on':''}" data-f="${x[0]}">${x[1]}</button>`).join('')+'<span style="width:1px;height:18px;background:var(--edge);margin:0 4px"></span>'+PLS.map(p=>`<button class="mp-chip${M.f==='pl:'+p.id?' on':''}" data-f="pl:${p.id}">${icon('list')} ${esc(p.name)} · ${p.ids.length}</button>`).join('')+'<button class="mp-chip" data-newpl>＋ 新建歌单</button>'+(M.q?`<span class="badge2 ai">搜索：${esc(M.q)}</span>`:'');
  const cp=curPl(),bar=$('#plbar');bar.hidden=!cp;if(cp)bar.innerHTML=`<b>${esc(cp.name)}</b><span class="badge2">${cp.ids.length} 首</span><button class="mp-chip" data-plact="play">${icon('play')} 播放</button><button class="mp-chip" data-plact="rename">重命名</button><button class="mp-chip" data-plact="del">删除歌单</button>`;
  $('#plside').innerHTML=PLS.map(p=>`<button data-plgo="${p.id}" class="${M.f==='pl:'+p.id&&document.body.dataset.view==='music'?'on':''}">${icon('list')}<span>${esc(p.name)}</span><small>${p.ids.length}</small></button>`).join('')+`<button data-plgo="new">${icon('list')}<span style="color:var(--mp-sub)">＋ 新建歌单</span></button>`;
  const l=mFiltered();$('#mcount').textContent=`共 ${MP.songs.filter(s=>!s.trial&&s.trial!==0).length} 首（演示数据），当前显示 ${l.length} 首 · 点卡片上的播放键可以直接播放整组`;
  const row=(s,n)=>`<div class="mp-row mrow${!MP.state.file&&s.id===MP.state.idx?' on':''}" data-i="${s.id}"><span class="mp-n">${String(n).padStart(2,'0')}</span><span class="mp-cover" style="background-image:${coverCSS(s.hue)}"></span><span class="mp-t">${s.t}<small>${s.a}</small></span><span class="mp-al">${s.al}</span><span class="mp-fmt">${s.fmt}</span><span class="pc">${s.plays?s.plays+' 次':'未播放'}</span><span class="mp-d">${fmtT(s.dur)}</span><span class="ra"><button class="${FAV.has(s.id)?'fav':''}" data-fav="${s.id}" title="收藏">${icon('heart')}</button><button data-pladd="${s.id}" title="${cp?'歌单…':'加入歌单'}">${icon('list')}</button><button data-cv="${s.id}" title="转换格式">${icon('convert')}</button></span></div>`;
  let h='';
  if(!l.length)h=`<div class="empty">${cp?'这个歌单还是空的。去“全部歌曲”里，把鼠标移到歌曲上点“加入歌单”。':'没有符合条件的歌曲'}</div>`;
  else if(M.g==='none')h=l.map((s,i)=>row(s,i+1)).join('');
  else{const key=s=>M.g==='fmt'?s.fmt.split(' ')[0]:s[M.g],gs={};l.forEach(s=>(gs[key(s)]=gs[key(s)]||[]).push(s));
    h=Object.keys(gs).sort((a,b)=>a.localeCompare(b,'zh')).map(k=>`<div class="grp">${k} · ${gs[k].length} 首 <button class="mp-chip" data-gplay="${esc(k)}">${icon('play')} 播放</button></div>`+gs[k].map((s,i)=>row(s,i+1)).join('')).join('')}
  $('#mlist').innerHTML=h;
}
function playSet(list,name){if(!list.length)return toast('这个分组里还没有歌曲');MP.setTrack(list[0].id,true);toast(`正在播放「${name}」· ${list.length} 首`)}
$('[data-view="music"]').addEventListener('click',e=>{
  const b=e.target.closest('[data-play],[data-f],[data-fav],[data-cv],[data-gplay],[data-newpl],[data-pladd],[data-plact]');if(!b)return;
  if(b.hasAttribute('data-newpl'))return newPl();
  if(b.dataset.pladd){e.stopPropagation();return plMenu(b,+b.dataset.pladd)}
  if(b.dataset.plact){const p=curPl();if(!p)return;if(b.dataset.plact==='play')return playSet(mFiltered(),p.name);
    if(b.dataset.plact==='rename')return askName('重命名歌单',p.name,n=>{p.name=n;savePl();mRender()});
    PLS=PLS.filter(x=>x!==p);savePl();M.f='all';mRender();return toast(`已删除歌单「${p.name}」（歌曲文件不受影响）`)}
  if(b.dataset.play){e.stopPropagation();const x=SMART.find(x=>x[0]===b.dataset.play);const keep=M.f;M.f=x[0];const l=mFiltered();M.f=keep;return playSet(l,x[1])}
  if(b.dataset.gplay!=null){e.stopPropagation();const k=b.dataset.gplay;return playSet(mFiltered().filter(s=>(M.g==='fmt'?s.fmt.split(' ')[0]:s[M.g])===k),k)}
  if(b.dataset.fav){e.stopPropagation();const i=+b.dataset.fav;FAV.has(i)?FAV.delete(i):FAV.add(i);toast(FAV.has(i)?'已加入收藏':'已取消收藏');return mRender()}
  if(b.dataset.cv){e.stopPropagation();return quickConvert(MP.songs[+b.dataset.cv])}
  if(b.dataset.f){M.f=b.dataset.f;mRender()}
});
$('#plside').onclick=e=>{const b=e.target.closest('[data-plgo]');if(!b)return;if(b.dataset.plgo==='new')return newPl();M.f='pl:'+b.dataset.plgo;MP.setView('music');mRender()};
$('#msort').onchange=e=>{M.sort=e.target.value;mRender()};$('#mgrp').onchange=e=>{M.g=e.target.value;mRender()};
MP.on('track',()=>{const c=MP.cur();if(c.plays!=null){c.plays++;c.last=0}});

/* ---------- 3. 全局搜索 ---------- */
const gs=$('#gs'),res=$('#results');
function search(){
  const q=gs.value.trim().toLowerCase();if(!q){res.classList.remove('open');if(M.q){M.q='';mRender()}return}
  const hit=(s,...f)=>f.some(x=>String(x).toLowerCase().includes(q));
  const sec=(name,items)=>items.length?`<h6>${name}</h6>`+items.join(''):'';
  const h=sec('歌曲',MP.songs.filter(s=>hit(s,s.t,s.a,s.al)).slice(0,5).map(s=>`<div data-r="song:${s.id}"><i style="background-image:${coverCSS(s.hue)}"></i>${s.t}<small>${s.a}</small></div>`))
   +sec('番剧',MP.anime.map((a,i)=>[a,i]).filter(([a])=>hit(a,a.t)).map(([a,i])=>`<div data-r="anime:${i}"><i style="background-image:${coverCSS(a.hue)}"></i>${a.t}<small>${a.s}</small></div>`))
   +sec('漫画',MANGA.map((a,i)=>[a,i]).filter(([a])=>hit(a,a.t)).map(([a,i])=>`<div data-r="manga:${i}"><i style="background-image:${coverCSS(a.hue,1)}"></i>${a.t}<small>${a.s}</small></div>`))
   +sec('视频',MP.videos.map((a,i)=>[a,i]).filter(([a])=>hit(a,a.t)).map(([a,i])=>`<div data-r="video:${i}"><i style="background-image:${coverCSS(a.hue,1)}"></i>${a.t}<small>${a.s}</small></div>`))
   +sec('小说',MP.novels.map((a,i)=>[a,i]).filter(([a])=>hit(a,a.t)).map(([a,i])=>`<div data-r="novel:${i}"><i style="background-image:${coverCSS(a.hue,1)}"></i>${a.t}<small>${a.s}</small></div>`));
  res.innerHTML=h?h+`<h6>操作</h6><div data-r="filter">${icon('music')} 在音乐库中筛选“${esc(gs.value.trim())}”</div>`:'<div class="empty" style="padding:18px">没有找到相关内容</div>';res.classList.add('open');
}
gs.oninput=search;gs.onfocus=search;
gs.onkeydown=e=>{if(e.key==='Enter'){const f=$('[data-r]',res);if(f)f.click()}else if(e.key==='Escape'){gs.value='';search();gs.blur()}};
res.onclick=e=>{const d=e.target.closest('[data-r]');if(!d)return;const [k,i]=d.dataset.r.split(':');res.classList.remove('open');
  if(k==='song')MP.setTrack(+i,true);else if(k==='anime'){MP.setView('anime');openSeries(+i)}else if(k==='manga')openReader(+i);
  else if(k==='video')MP.openVideo(MP.videos[+i]);else if(k==='novel')MP.openNovel(MP.novels[+i]);
  else{M.q=gs.value.trim().toLowerCase();M.f='all';MP.setView('music');mRender()}
  if(k!=='filter')gs.value=''};
document.addEventListener('pointerdown',e=>{if(!e.target.closest('.topbar'))res.classList.remove('open')});
document.addEventListener('keydown',e=>{if(e.key==='/'&&!/INPUT|TEXTAREA|SELECT/.test(e.target.tagName)){e.preventDefault();gs.focus()}});

/* ---------- 4. 番剧：剧集式详情 ---------- */
const EPT=['起雾的夜晚','第一封信','天台与气象图','不会响的电话','旧书店的二楼','潮汐时刻表','迟到的回信','雨停之前','屋顶上的约定','末班渡轮','星屑','再见，以及你好','番外 · 夏日祭'];
const SYN=['港口小镇有一家只在夜里营业的邮局，据说寄出的信总能送到想见的人手里。转学生第一次推开那扇门的晚上，海上起了很大的雾。','天台上的旧气象站还在运转。几个放学后无处可去的人，决定每天记录一次这座城市的天气。','一间从不打烊的旧书店，一台会自己翻页的机器人，以及每个深夜来访的客人各自带来的故事。'];
const aiDone=new Set();                       // 已经用 AI 生成过字幕的“番剧:集”
const subOf=(si,e)=>aiDone.has(si+':'+e)?'ai':si%3===0?'ok':si%3===1?(e%3===2?'no':'ok'):'no';
const SUBL={ok:'内封简中',ai:'AI 字幕',no:'无字幕'};
MP.anime.forEach((a,i)=>{a.seasons=i%3===1?2:1;a.syn=SYN[i%3]});
let AF='all';
function aRender(){
  $('#afilter').innerHTML=[['all','全部'],['on','在追'],['done','已看完'],['new','未看'],['nosub','缺字幕']].map(x=>`<button class="mp-chip${AF===x[0]?' on':''}" data-af="${x[0]}">${x[1]}</button>`).join('');
  const ok=(a,i)=>AF==='all'||(AF==='on'&&a.seen>0&&a.seen<a.ep)||(AF==='done'&&a.seen>=a.ep)||(AF==='new'&&!a.seen)||(AF==='nosub'&&i%3!==0);
  $('#agrid').innerHTML=MP.anime.map((a,i)=>ok(a,i)?`<div class="mp-card" data-series="${i}"><div class="mp-poster" style="background-image:${coverCSS(a.hue)}"><span class="mp-badge">${a.seen>=a.ep?'已看完':a.seen?'看到第 '+a.seen+' 集':'未观看'}</span><span class="mp-pt">${a.t}</span><i class="mp-bar" style="width:${a.seen/a.ep*100}%"></i></div><div class="mp-card-t">${a.t}</div><div class="mp-card-s">${a.s}${i%3?' · 部分无字幕':''}</div></div>`:'').join('')||'<div class="empty">没有符合条件的番剧</div>';
}
function openSeries(i,season=1){
  const a=MP.anime[i],next=Math.min(a.seen+1,a.ep),nosub=[...Array(a.ep).keys()].filter(e=>subOf(i,e)==='no').length;
  $('#alist').style.display='none';const d=$('#adetail');d.classList.add('on');
  d.innerHTML=`<button class="mp-chip" data-aback>← 返回番剧列表</button>
  <div class="banner" style="margin-top:12px;background-image:${coverCSS(a.hue)}"><div class="pst" style="background-image:${coverCSS(a.hue,1)}"></div><div>
    <h2>${a.t}</h2><div class="acts" style="margin-bottom:10px"><span class="badge2">${a.s}</span><span class="badge2">1080p · HEVC 10bit</span><span class="badge2">日语 FLAC 2.0</span><span class="badge2 ${nosub?'no':'ok'}">${nosub?nosub+' 集缺字幕':'字幕齐全'}</span></div>
    <p>${a.syn}</p><div class="acts"><button class="mp-btn" data-ep="${next-1}">${icon('play')} ${a.seen?(a.seen>=a.ep?'重新观看':'继续观看 第 '+next+' 集'):'开始观看'}</button>
    ${nosub?`<button class="mp-btn ghost" data-aiall>${icon('eq')} 为缺字幕的 ${nosub} 集生成 AI 字幕</button>`:''}<button class="mp-btn ghost" data-mark>标记为已看完</button></div></div></div>
  <div class="tabs">${[...Array(a.seasons).keys()].map(s=>`<button class="mp-chip${s+1===season?' on':''}" data-season="${s+1}">第 ${s+1} 季</button>`).join('')}<span class="badge2">共 ${a.ep} 集 · 已看 ${a.seen} 集</span></div>
  <div class="eps">${[...Array(a.ep).keys()].map(e=>{const st=subOf(i,e),p=e<a.seen?1:e===a.seen?.35:0;return `<div class="ep" data-ep="${e}"><div class="th" style="background-image:${coverCSS((a.hue+e*23+season*40)%360,e%2)}"><b>${String(e+1).padStart(2,'0')}</b><em>${icon('play')}</em><u>23:${String(40+e%19).padStart(2,'0')}</u><i style="width:${p*100}%"></i></div><p>第 ${e+1} 话　${EPT[(e+season*3)%EPT.length]}</p><div class="row"><span class="badge2 ${st}">${SUBL[st]}</span>${e<a.seen?'<span class="badge2">已看</span>':''}${st==='no'?`<button data-aiep="${e}">生成字幕</button>`:''}</div></div>`}).join('')}</div>`;
  d.onclick=ev=>{const b=ev.target.closest('[data-aback],[data-aiep],[data-aiall],[data-mark],[data-season],[data-ep]');if(!b)return;
    if(b.hasAttribute('data-aback')){d.classList.remove('on');$('#alist').style.display='';return aRender()}
    if(b.dataset.aiep!=null){ev.stopPropagation();return gotoSub(`${a.t} · 第 ${+b.dataset.aiep+1} 话`,i+':'+b.dataset.aiep,i)}
    if(b.hasAttribute('data-aiall'))return gotoSub(`${a.t} · 缺字幕的 ${nosub} 集（批量）`,'all:'+i,i);
    if(b.hasAttribute('data-mark')){a.seen=a.ep;toast('已标记为看完');return openSeries(i,season)}
    if(b.dataset.season)return openSeries(i,+b.dataset.season);
    const e=+b.dataset.ep;a.seen=Math.max(a.seen,Math.min(e+1,a.ep));MP.openVideo({t:`${a.t} · 第 ${e+1} 话 ${EPT[(e+season*3)%EPT.length]}`,fmt:'1080p · HEVC 10bit',hue:a.hue,p:.05});openSeries(i,season)};
  $('[data-view="anime"]').scrollTop=0;
}
$('#alist').addEventListener('click',e=>{const f=e.target.closest('[data-af]');if(f){AF=f.dataset.af;return aRender()}const c=e.target.closest('[data-series]');if(c)openSeries(+c.dataset.series)});

/* ---------- 5. AI 服务设置（演示：不保存、不联网） ---------- */
let aiCfg=null;
function aiNote(){const t=aiCfg?`已填写 AI 服务：${esc(aiCfg.name)}。demo 不会真的调用接口，下面展示的是示例结果。`:'还没有填写 AI 服务的 API 密钥。demo 里可以直接看示例效果；正式版需要先 <a data-aiset>填写密钥</a>，或者改用本地模型。';
  $$('#snote,.mnote').forEach(n=>n.innerHTML=t)}
function openAiSet(){
  const inp='style="flex:1.4;background:var(--mp-panel2);color:inherit;border:1px solid var(--mp-line);border-radius:8px;padding:6px 9px;font:inherit;font-size:13px;min-width:0"';
  const m=MP.modal('mp-ai',`<h3>${icon('gear')} AI 服务<button class="mp-x">✕</button></h3>
  <p class="mp-desc">AI 字幕和漫画翻译需要语音识别和大语言模型。填一次，全局可用；也可以完全不填，改用本地模型（免费、离线，但更慢，需要较好的显卡）。</p>
  <label class="mp-opt"><div>服务类型<small>兼容 OpenAI 接口格式的服务都可以接入</small></div><select id="ai-p"><option>OpenAI 兼容接口</option><option>自定义接口</option><option>仅使用本地模型</option></select></label>
  <label class="mp-opt"><div>接口地址</div><input id="ai-u" ${inp} placeholder="https://api.example.com/v1"></label>
  <label class="mp-opt"><div>API 密钥<small>demo 不保存也不发送，请不要填真实密钥</small></div><input id="ai-k" type="password" ${inp} placeholder="sk-…" autocomplete="off"></label>
  <label class="mp-opt"><div>语音识别模型<small>用于 AI 字幕</small></div><input id="ai-m1" ${inp} placeholder="例如 whisper 系列"></label>
  <label class="mp-opt"><div>翻译模型<small>用于字幕翻译和漫画翻译</small></div><input id="ai-m2" ${inp} placeholder="任意对话模型"></label>
  <label class="mp-opt"><div>图像识别模型<small>用于识别漫画里的文字，可留空改用本地 OCR</small></div><input id="ai-m3" ${inp} placeholder="支持图片输入的模型"></label>
  <label class="mp-opt"><div>每月费用上限提醒<small>正式版会在开始任务前估算用量，超过上限时先询问</small></div><select><option>不提醒</option><option selected>约 ¥20</option><option>约 ¥50</option><option>约 ¥100</option></select></label>
  <div class="mp-opt" style="display:block"><small style="display:block;line-height:1.7">隐私说明：正式版只会把完成任务必需的内容（视频的音轨、漫画页面图片、待翻译的文字）发送到你填写的服务地址，不经过任何第三方服务器；密钥用 Windows 凭据管理器加密保存在本机。</small></div>
  <div style="margin-top:14px;display:flex;gap:10px;align-items:center"><button class="mp-btn" id="ai-s">保存</button><button class="mp-btn ghost" id="ai-t">测试连接</button><span id="ai-r" style="font-size:12.5px;color:var(--mp-sub)"></span></div>`);
  $('#ai-t',m).onclick=async()=>{const r=$('#ai-r',m);r.textContent='正在测试…';await sleep(900);r.textContent='✓ 演示：连接正常（demo 没有真的发出请求）'};
  $('#ai-s',m).onclick=()=>{aiCfg={name:$('#ai-p',m).value+($('#ai-u',m).value?' · '+$('#ai-u',m).value.replace(/^https?:\/\//,'').split('/')[0]:'')};m.classList.remove('open');aiNote();toast('已保存（仅本次演示有效）')};
}
$('#aiset').onclick=openAiSet;
document.addEventListener('click',e=>{if(e.target.closest('[data-aiset]'))openAiSet()});

/* ---------- 6. AI 字幕：识别、断句、对齐、翻译 ---------- */
const SUBS=[[1.2,3.4,'ねえ、聞いた？港の郵便局のこと','喂，听说了吗？港口那家邮局的事。'],[4.1,6.6,'夜しか開かないんだって','据说只在夜里开门。'],[7.4,8.6,'まさか','怎么可能。'],[9.5,12.1,'本当だよ。霧の夜だけ、灯りがつくの','是真的。只有起雾的夜晚，灯才会亮。'],
 [13.0,15.2,'じゃあ、今夜行ってみよう','那今晚就去看看吧。'],[16.3,17.6,'え、今から！？','诶，现在就去！？'],[18.4,21.3,'返事を待っている人がいるんだ','因为有人在等一封回信。'],[22.2,24.6,'…わかった。傘、持っていくね','……好吧。我带上伞。']];
const STEPS=['提取音轨','检测人声','语音识别','断句与对齐','翻译','校对并输出'];const TL=26;
const sub={step:-1,rec:0,align:0,head:0,done:false,key:null,si:null};
const env=Array.from({length:520},(_,i)=>{const t=i/520*TL,inS=SUBS.some(s=>t>=s[0]&&t<=s[1]);return (inS?.35+.65*Math.abs(Math.sin(i*1.7)*Math.sin(i*.37)):.04+.05*Math.abs(Math.sin(i*.9)))});
const off=SUBS.map((_,i)=>[(i%2?.5:-.45),(i%3?.6:-.3)]);   // 对齐前的时间偏差（秒）
function fileOpts(){const o=[];MP.videos.forEach(v=>o.push(v.t+'　'+v.fmt));MP.anime.forEach((a,i)=>{if(i%3)o.push(a.t+' · 缺字幕的剧集（批量）')});return o}
$('#sfile').innerHTML=fileOpts().map(x=>`<option>${x}</option>`).join('');
$('#ssteps').innerHTML=STEPS.map(s=>`<span class="st">${s}</span>`).join('');
function gotoSub(name,key,si){MP.setView('aisub');const s=$('#sfile');let o=[...s.options].find(o=>o.textContent===name);if(!o){o=new Option(name);s.add(o,0)}s.value=o.value;sub.key=key;sub.si=si;toast('已选中：'+name)}
MP.canvas($('#tl'),(g,w,h,t)=>{g.clearRect(0,0,w,h);const x=s=>s/TL*w,mid=h*.42;
  g.fillStyle=sub.step>=1?'#ffffff12':'transparent';if(sub.step>=1)SUBS.forEach(s=>g.fillRect(x(s[0]),4,x(s[1])-x(s[0]),h*.76));       // 人声区间
  g.strokeStyle='#ffffff66';g.lineWidth=1;g.beginPath();env.forEach((v,i)=>{const px=i/env.length*w,a=sub.step>=0?v*h*.34:2;g.moveTo(px,mid-a);g.lineTo(px,mid+a)});g.stroke();
  const acc=getComputedStyle(document.documentElement).getPropertyValue('--mp-accent')||'#7aa2ff';
  SUBS.forEach((s,i)=>{if(i>=sub.rec)return;const k=1-sub.align,a=s[0]+off[i][0]*k,b=s[1]+off[i][1]*k;
    g.fillStyle=sub.align>=1?'#7be0a3':acc;g.globalAlpha=.85;g.beginPath();g.roundRect(x(a),h*.82,Math.max(4,x(b)-x(a)),h*.13,4);g.fill();g.globalAlpha=1;
    if(sub.align<1&&sub.step>=3){g.strokeStyle='#7be0a3';g.setLineDash([3,3]);g.strokeRect(x(s[0]),h*.8,x(s[1])-x(s[0]),h*.17);g.setLineDash([])}});
  if(sub.step>=0&&!sub.done){g.strokeStyle='#fff';g.beginPath();g.moveTo(x(sub.head),0);g.lineTo(x(sub.head),h);g.stroke()}
  g.fillStyle='#ffffff80';g.font='10px Consolas,monospace';for(let s=0;s<=TL;s+=5)g.fillText('0:'+String(s).padStart(2,'0'),x(s)+3,h-3)});
const tc=s=>`0:${String(s|0).padStart(2,'0')}.${String(Math.round(s%1*10))}`;
async function runSub(){
  const btn=$('#sgo');if(btn.disabled)return;btn.disabled=true;btn.textContent='处理中…';const st=$$('#ssteps .st'),box=$('#subt'),mode=$('#smode').value;
  Object.assign(sub,{step:-1,rec:0,align:0,head:0,done:false});box.innerHTML='';st.forEach(s=>s.className='st');
  const step=async(i,fn)=>{sub.step=i;st[i].classList.add('run');await fn();st[i].className='st ok'};
  const sweep=async(ms,each)=>{const t0=performance.now();let last=-1;while(true){const p=Math.min(1,(performance.now()-t0)/ms);sub.head=p*TL;if(each){const k=SUBS.filter(s=>s[1]<=sub.head).length;if(k!==last){last=k;each(k)}}if(p>=1)break;await sleep(30)}};
  await step(0,()=>sweep(700));await step(1,()=>sweep(800));
  await step(2,()=>sweep(2600,k=>{while(sub.rec<k){const s=SUBS[sub.rec],i=sub.rec++;const a=s[0]+off[i][0],b=s[1]+off[i][1];
    box.insertAdjacentHTML('beforeend',`<div class="sl" data-i="${i}"><code>${tc(a)} → ${tc(b)}</code><input value="${esc(s[2])}"><input class="zh" placeholder="${mode==='src'?'（不翻译）':'等待翻译…'}"></div>`);box.scrollTop=1e5}}));
  await step(3,async()=>{const t0=performance.now();while(sub.align<1){sub.align=Math.min(1,(performance.now()-t0)/900);await sleep(30)}
    $$('.sl',box).forEach((r,i)=>$('code',r).textContent=`${tc(SUBS[i][0])} → ${tc(SUBS[i][1])}`)});
  await step(4,async()=>{if(mode==='src')return;for(const r of $$('.sl',box)){$('.zh',r).value=SUBS[+r.dataset.i][3];await sleep(170)}});
  await step(5,()=>sleep(600));
  sub.done=true;btn.disabled=false;btn.textContent='重新生成';const lb=$('#sload');lb.disabled=false;lb.style.opacity=1;
  if(sub.key){if(sub.key.startsWith('all:')){const si=+sub.key.slice(4);for(let e=0;e<MP.anime[si].ep;e++)aiDone.add(si+':'+e)}else aiDone.add(sub.key);if($('#adetail').classList.contains('on'))openSeries(sub.si)}
  toast('字幕已生成并对齐，可以逐句修改，或直接在播放器中预览');
}
$('#sgo').onclick=runSub;$('#sload').onclick=()=>MP.openVideo({t:$('#sfile').value.split('　')[0],fmt:'AI 字幕 · 双语',hue:232,p:0});

/* ---------- 7. 格式转换：完成后回写 ---------- */
const JOBS=[['雾港.flac','FLAC → MP3 320k','38.2 MB','8.9 MB','a',0],['潮汐之间.2160p.mkv','MKV → MP4（仅换封装）','14.6 GB','14.6 GB','v',0],['演唱会实录.ts','提取音轨 → FLAC','2.1 GB','486 MB','a',5],['已购单曲.mflac','加密格式 → FLAC（插件）','31.0 MB','30.8 MB','a',1],['家庭录像_0412.mov','MOV → MP4 H.265','820 MB','212 MB','v',7]];
const AFTER={lib:['加入媒体库，保留原文件','新文件保存到输出目录并自动加入媒体库，原文件不动。'],replace:['替换原文件','新文件接替原文件在媒体库里的位置，播放次数、收藏、歌单、观看进度都保留。原文件先移到回收站，随时可以撤销。'],
 side:['放在原文件旁边','新文件和原文件放在同一个文件夹，两者都在库里，播放时可以选用哪一个版本。'],dir:['只输出到指定目录','只生成文件，不改动媒体库。输出目录：D:\\Media\\Converted']};
let after='lib';
$('#cpre').innerHTML=['MP3','FLAC','AAC','Opus','MP4','仅换封装','提取音轨','视频转 GIF','压制字幕'].map((x,i)=>`<button class="mp-chip${i?'':' on'}">${x}</button>`).join('');
$('#cpre').onclick=e=>{const c=e.target.closest('.mp-chip');if(c){$$('#cpre .mp-chip').forEach(x=>x.classList.remove('on'));c.classList.add('on')}};
function cRender(){$('#cnote').textContent=AFTER[after][1];$('#cq').innerHTML=JOBS.map((j,i)=>`<div class="mp-qi" data-j="${i}" style="--p:0"><span>${j[0]}</span><small class="mp-qs">等待中</small><small>${j[1]} · ${j[2]}</small><span></span><div class="mp-pb"><i></i></div><div class="res"></div></div>`).join('')}
$('#cafter').onclick=e=>{const b=e.target.closest('[data-a]');if(!b)return;after=b.dataset.a;$$('#cafter button').forEach(x=>x.classList.toggle('on',x===b));cRender()};
function resLine(j,mode){const save=parseFloat(j[2])>parseFloat(j[3])&&j[2].slice(-2)===j[3].slice(-2)?`，节省 ${(parseFloat(j[2])-parseFloat(j[3])).toFixed(1)} ${j[2].slice(-2)}`:'';
  return ({lib:`✓ 已加入媒体库${save}`,replace:`✓ 已替换原文件${save}，原文件在回收站`,side:'✓ 已保存到原文件旁边',dir:'✓ 已输出到 D:\\Media\\Converted'})[mode]
   +` <button data-act="play">${j[4]==='a'?'播放':'观看'}</button>`+(mode!=='dir'?'<button data-act="show">在库中显示</button>':'<button data-act="show">打开文件夹</button>')+(mode==='replace'?'<button data-act="undo">撤销替换</button>':mode!=='dir'&&mode!=='side'?'<button data-act="rep">改为替换原文件</button>':'')}
$('#cgo').onclick=async e=>{const b=e.currentTarget;if(b.disabled)return;b.disabled=true;cRender();const mode=after;
  for(const it of $$('#cq .mp-qi')){const j=JOBS[+it.dataset.j],st=$('.mp-qs',it);
    if(j[0].endsWith('.mflac')&&!$('#cplug').checked){st.textContent='已跳过：未启用插件';continue}
    for(let p=0;p<1;p+=.03+Math.random()*.06){it.style.setProperty('--p',Math.min(p,1));st.textContent=Math.round(p*100)+'% · '+(40+Math.random()*60|0)+'× 速度';await sleep(70)}
    it.style.setProperty('--p',1);st.textContent='完成';it.classList.add('done');it.dataset.mode=mode;$('.res',it).innerHTML=resLine(j,mode)}
  b.disabled=false;toast('转换完成 · '+AFTER[mode][0])};
$('#cq').onclick=e=>{const b=e.target.closest('[data-act]');if(!b)return;const it=b.closest('.mp-qi'),j=JOBS[+it.dataset.j],a=b.dataset.act;
  if(a==='play')return j[4]==='a'?(MP.setTrack(j[5],true),toast('正在播放转换后的文件')):MP.openVideo({t:j[0].replace(/\.\w+$/,''),fmt:j[1].split('→ ')[1],hue:205,p:0});
  if(a==='show')return toast('正式版会跳到这个文件所在的位置');
  if(a==='undo'){it.dataset.mode='lib';$('.res',it).innerHTML=resLine(j,'lib');return toast('已撤销：原文件已从回收站恢复')}
  if(a==='rep'){it.dataset.mode='replace';$('.res',it).innerHTML=resLine(j,'replace');return toast('已替换原文件，可随时撤销')}};
/* 播放界面 / 歌曲行上的快捷转换 */
const pop=$('#pop');
function quickConvert(s){s=s||MP.cur();const ext=s.fmt.split(' ')[0].toLowerCase();
  pop.innerHTML=`<h5>快捷转换</h5><p>${esc(s.t)}.${ext==='ogg'?'ogg':ext==='dsd64'?'dsf':ext} · ${s.fmt}</p>
  <div class="chips" id="qf" style="margin-bottom:10px">${['MP3 320k','AAC 256k','Opus 160k','FLAC','WAV'].filter(f=>!s.fmt.startsWith(f.split(' ')[0])).map((f,i)=>`<button class="mp-chip${i?'':' on'}">${f}</button>`).join('')}</div>
  <div class="frow" style="margin-bottom:12px"><span style="min-width:0">完成后</span><select class="sel" id="qa" style="flex:1">${Object.keys(AFTER).map(k=>`<option value="${k}" ${k==='replace'?'selected':''}>${AFTER[k][0]}</option>`).join('')}</select></div>
  <div class="mp-qi" id="qp" style="--p:0;display:none"><span>转换中</span><small class="mp-qs"></small><div class="mp-pb"><i></i></div></div>
  <div style="display:flex;gap:8px;margin-top:10px"><button class="mp-btn" id="qgo">开始</button><button class="mp-btn ghost" id="qx">关闭</button></div>`;
  pop.classList.add('open');$('#qx').onclick=()=>pop.classList.remove('open');
  $('#qf').onclick=e=>{const c=e.target.closest('.mp-chip');if(c){$$('#qf .mp-chip').forEach(x=>x.classList.remove('on'));c.classList.add('on')}};
  $('#qgo').onclick=async e=>{const fmt=$('#qf .on').textContent,mode=$('#qa').value,q=$('#qp');e.currentTarget.disabled=true;q.style.display='';
    for(let p=0;p<1;p+=.05+Math.random()*.08){q.style.setProperty('--p',p);$('.mp-qs',q).textContent=Math.round(p*100)+'%';await sleep(80)}
    q.style.setProperty('--p',1);$('.mp-qs',q).textContent='完成 ✓';if(mode==='replace'||mode==='side'){s.fmt=fmt;mRender();if(s===MP.cur())$$('[data-mp="fmt"]').forEach(x=>x.textContent=fmt)}
    toast(`已转为 ${fmt} · ${AFTER[mode][0]}`+(s===MP.cur()&&MP.state.playing?'，播放不受影响':''));setTimeout(()=>pop.classList.remove('open'),900)};
}
$('#qc').onclick=()=>pop.classList.contains('open')?pop.classList.remove('open'):quickConvert();

/* ---------- 8. 漫画：书架、阅读器、AI 翻译嵌字 ---------- */
const MANGA=[{t:'雾港邮局',s:'第 1 卷 · 日文原版',hue:205,p:.2,lang:'日文'},{t:'天台气象站',s:'全 6 卷 · 日文原版',hue:186,p:0,lang:'日文'},{t:'旧书店的机器人',s:'连载中 · 英文版',hue:338,p:.6,lang:'英文'},
 {t:'猫耳侦探事务所',s:'全 3 卷 · 中文版',hue:52,p:1,lang:'中文'},{t:'钢之候鸟 前传',s:'单行本 · PDF',hue:8,p:0,lang:'日文'},{t:'便当日和',s:'全 4 卷 · 中文版',hue:30,p:.45,lang:'中文'}];
$('#mgrid').innerHTML=MANGA.map((m,i)=>`<div class="mp-card mp-book" data-manga="${i}"><div class="mp-poster" style="background-image:${coverCSS(m.hue,1)};filter:saturate(.75)"><span class="mp-badge">${m.lang}${m.lang==='中文'?'':' · 可翻译'}</span><span class="mp-pt">${m.t}</span><i class="mp-bar" style="width:${m.p*100}%"></i></div><div class="mp-card-t">${m.t}</div><div class="mp-card-s">${m.s}</div></div>`).join('');
$('#mgrid').onclick=e=>{const c=e.target.closest('[data-manga]');if(c)openReader(+c.dataset.manga)};
const SKY='linear-gradient(#9a9a9a,#e8e8e8)',NIGHT='linear-gradient(#2a2a2e,#77777c)',WALL='linear-gradient(90deg,#d8d8d8,#f2f2f2)',FOG='radial-gradient(circle at 50% 120%,#fff,#b8b8b8 70%)',DOOR='linear-gradient(#444,#999 60%,#ccc)';
/* 每页：网格模板 + 分格；气泡坐标是分格内的百分比。对白为原创内容。 */
const PAGES=[
 {gt:"'a a' 1.15fr 'b c' 1fr / 1fr 1fr",pn:[{ga:'a',art:SKY,figs:[['',8,16,34],['l',28,15,30]],bb:[[58,10,24,62,'ねえ、聞いた？','喂，听说了吗？']]},
   {ga:'b',art:WALL,figs:[['l',12,34,52]],bb:[[52,8,40,70,'港の郵便局、夜しか開かないって','港口那家邮局，听说只在夜里开门。']]},{ga:'c',art:WALL,figs:[['',52,34,50]],bb:[[10,12,34,48,'まさか…','怎么可能……']]}]},
 {gt:"'a b' 1fr 'c c' 1.1fr / 1.2fr 1fr",pn:[{ga:'a',art:NIGHT,figs:[['l',10,30,48]],bb:[[52,8,36,58,'行ってみよう','去看看吧。']]},{ga:'b',art:NIGHT,figs:[['',50,38,50]],bb:[[8,8,38,60,'え、今から！？','诶，现在就去！？']]},
   {ga:'c',art:FOG,figs:[['l',14,14,40],['',32,13,36]],bb:[[62,12,24,66,'霧が出る前に','趁雾还没起来。']]}]},
 {gt:"'a a' 1fr 'b c' 1.3fr / 1fr 1.2fr",pn:[{ga:'a',art:DOOR,figs:[['',40,16,50]],bb:[[66,14,22,60,'…誰かいる','……有人在。']]},
   {ga:'b',art:WALL,figs:[['l',48,40,54]],bb:[[6,6,40,64,'いらっしゃい。手紙かな？','欢迎光临。是来寄信的吗？']]},{ga:'c',art:WALL,figs:[['',10,30,46]],bb:[[48,8,42,66,'返事を、待っているんです','我在等一封回信。',"“返事”结合前文的“手紙”，译为“回信”而不是“回答”"]]}]},
 {gt:"'a b' 1.2fr 'c b' 1fr / 1fr 1fr",pn:[{ga:'a',art:WALL,figs:[['l',8,36,50]],bb:[[50,6,42,70,'それなら、まだ届いていないよ','那封信的话，还没有到哦。','补出省略的主语“那封信”，避免直译成“那样的话还没到”']]},
   {ga:'b',art:FOG,figs:[['l',20,46,44]],bb:[[18,6,56,40,'潮には潮の時刻表があるからね','毕竟潮水有潮水的时刻表。']]},{ga:'c',art:NIGHT,figs:[['',56,30,52]],bb:[[8,10,38,62,'…また明日、来ます','……我明天再来。','敬体“来ます”保持礼貌语气，与角色此前的说话方式一致']]}]}];
const R={m:0,page:0,mode:'single',rtl:true,view:'jp',done:false,busy:false,panel:true};let BB=[];
function openReader(i){R.m=i;R.page=0;R.done=false;R.busy=false;R.view='jp';const m=MANGA[i],rd=$('#reader');
  let k=0;BB=[];
  rd.innerHTML=`<div class="rtop"><button class="rbtn" data-r="close">← 书架</button><b>${m.t}</b><span style="color:#999">第 1 话 · 夜里的邮局</span><span class="sp"></span>
   <button class="rbtn" data-r="prev">上一页</button><span id="rpg" style="font-variant-numeric:tabular-nums;min-width:4.5em;text-align:center"></span><button class="rbtn" data-r="next">下一页</button>
   <button class="rbtn on" data-r="single">单页</button><button class="rbtn" data-r="double">双页</button><button class="rbtn" data-r="scroll">长条</button><button class="rbtn on" data-r="rtl" title="日漫从右往左翻页">右→左</button>
   <span style="width:1px;height:20px;background:#ffffff22"></span>
   <button class="rbtn on" data-r="jp" data-v>原文</button><button class="rbtn" data-r="zh" data-v disabled>译文</button><button class="rbtn" data-r="both" data-v disabled>对照</button>
   <button class="rbtn" data-r="panel">校对面板</button><button class="rbtn pri" data-r="go">${icon('eq')} AI 翻译本话</button></div>
  <div class="rbody"><div class="pages rtl" id="pages">${PAGES.map((p,pi)=>`<div class="page" data-p="${pi}" style="grid-template:${p.gt}"><div class="scanl"></div>${p.pn.map(n=>`<div class="pn" style="grid-area:${n.ga};--art:${n.art}">${n.figs.map(f=>`<div class="fig ${f[0]}" style="left:${f[1]}%;width:${f[2]}%;height:${f[3]}%"></div>`).join('')}${n.bb.map(b=>{BB.push({jp:b[4],zh:b[5],note:b[6],page:pi});return `<div class="bb" data-k="${k++}" style="left:${b[0]}%;top:${b[1]}%;width:${b[2]}%;height:${b[3]}%"><span>${b[4]}</span><small>${b[4]}</small></div>`}).join('')}</div>`).join('')}<span class="pgn">${pi+1}</span></div>`).join('')}</div>
   <div class="rside"><h5>AI 翻译</h5><div class="note mnote"></div><div class="steps" id="msteps">${['找出文字区域','识别原文','结合上下文翻译','擦除原文','嵌入译文'].map(s=>`<span class="st">${s}</span>`).join('')}</div>
    <h5>术语表 <span style="color:#888;font-weight:400;font-size:11.5px">保证人名、地名全书一致</span></h5><div class="gl"><span>港の郵便局 → 港口邮局</span><span>霧港 → 雾港</span><span>アキラ → 阿砚</span><span>＋ 添加</span></div>
    <h5>逐句校对 <span style="color:#888;font-weight:400;font-size:11.5px">改这里，画面同步更新</span></h5><div id="trs"><div class="empty" style="padding:14px">点右上角“AI 翻译本话”，这里会列出每个气泡的原文和译文。</div></div>
    <div style="display:flex;gap:8px;flex-wrap:wrap"><button class="rbtn" data-r="export" disabled>导出为新文件（保留原版）</button><button class="rbtn" data-r="redo" disabled>重新翻译选中句</button></div></div></div>`;
  rd.classList.add('open');aiNote();rSync();
}
function rSync(){const pg=$('#pages');if(!pg)return;pg.className='pages'+(R.mode==='double'?' double':R.mode==='scroll'?' scroll':'')+(R.rtl?' rtl':'')+(R.view==='both'?' both':'');
  $$('.page',pg).forEach((p,i)=>{p.classList.toggle('cur',i===R.page);p.classList.toggle('cur2',R.mode==='double'&&i===R.page+1)});
  $('#rpg').textContent=R.mode==='scroll'?`共 ${PAGES.length} 页`:R.mode==='double'?`${R.page+1}-${Math.min(R.page+2,PAGES.length)} / ${PAGES.length}`:`${R.page+1} / ${PAGES.length}`;
  $$('#reader [data-r]').forEach(b=>{const k=b.dataset.r;if(['single','double','scroll'].includes(k))b.classList.toggle('on',R.mode===k);if(k==='rtl')b.classList.toggle('on',R.rtl);if(b.hasAttribute('data-v')){b.classList.toggle('on',R.view===k);if(k!=='jp')b.disabled=!R.done}
    if(k==='panel')b.classList.toggle('on',R.panel);if(k==='export'||k==='redo')b.disabled=!R.done;if(k==='go'){b.disabled=R.busy;b.innerHTML=icon('eq')+(R.busy?' 翻译中…':R.done?' 重新翻译本话':' AI 翻译本话')}});
  $('.rbody').classList.toggle('nop',!R.panel);
  $$('#pages .bb').forEach(b=>{const d=BB[+b.dataset.k],sp=$('span',b),zh=R.done&&R.view!=='jp';sp.textContent=zh?d.zh:d.jp;sp.classList.toggle('zh',zh)});
}
const turn=d=>{const st=R.mode==='double'?2:1;R.page=Math.max(0,Math.min(PAGES.length-1,R.page+d*st));rSync()};
async function translate(){
  if(R.busy)return;R.busy=true;R.done=false;R.view='jp';R.panel=true;rSync();const st=$$('#msteps .st'),trs=$('#trs'),bbs=$$('#pages .bb'),pgs=$$('#pages .page');
  st.forEach(s=>s.className='st');trs.innerHTML='';bbs.forEach(b=>b.classList.remove('det','wipe'));
  const step=async(i,fn)=>{st[i].classList.add('run');await fn();st[i].className='st ok'};
  const show=p=>{if(R.mode!=='scroll'&&R.page!==p&&!(R.mode==='double'&&R.page+1===p)){R.page=R.mode==='double'?p-p%2:p;rSync()}};
  await step(0,async()=>{for(let p=0;p<PAGES.length;p++){show(p);pgs[p].classList.add('scan');await sleep(350);for(const b of bbs)if(BB[+b.dataset.k].page===p){b.classList.add('det');await sleep(150)}await sleep(250);pgs[p].classList.remove('scan')}});
  await step(1,async()=>{for(let k=0;k<BB.length;k++){trs.insertAdjacentHTML('beforeend',`<div class="tr" data-k="${k}"><small>第 ${BB[k].page+1} 页 · ${BB[k].jp}</small><input placeholder="…" data-k="${k}"></div>`);trs.lastChild.scrollIntoView({block:'nearest'});await sleep(110)}});
  await step(2,async()=>{for(let k=0;k<BB.length;k++){const r=$(`.tr[data-k="${k}"]`,trs),inp=$('input',r);for(let c=1;c<=BB[k].zh.length;c+=2){inp.value=BB[k].zh.slice(0,c);await sleep(14)}inp.value=BB[k].zh;if(BB[k].note)r.insertAdjacentHTML('beforeend',`<i>上下文：${BB[k].note}</i>`);r.scrollIntoView({block:'nearest'})}});
  show(0);
  await step(3,async()=>{bbs.forEach(b=>b.classList.add('wipe'));await sleep(650)});
  await step(4,async()=>{R.done=true;R.view='zh';rSync();bbs.forEach(b=>b.classList.remove('wipe','det'));await sleep(500)});
  R.busy=false;rSync();toast('翻译完成：可以切换 原文 / 译文 / 对照，或在右侧逐句修改');
}
$('#reader').addEventListener('click',e=>{const b=e.target.closest('[data-r]');
  if(!b){const tr=e.target.closest('.tr');if(tr&&e.target.tagName!=='INPUT'){const d=BB[+tr.dataset.k];R.page=R.mode==='double'?d.page-d.page%2:d.page;rSync();const el=$(`#pages .bb[data-k="${tr.dataset.k}"]`);el.classList.add('det');el.scrollIntoView({block:'nearest'});setTimeout(()=>el.classList.remove('det'),1200)}
    else if(e.target.closest('.page')&&R.mode!=='scroll'&&!e.target.closest('.bb')){const r=e.target.closest('.pages').getBoundingClientRect();const right=e.clientX>r.left+r.width/2;turn((right?1:-1)*(R.rtl?-1:1))}return}
  const k=b.dataset.r;
  if(k==='close')return $('#reader').classList.remove('open');if(k==='prev')return turn(-1);if(k==='next')return turn(1);
  if(['single','double','scroll'].includes(k)){R.mode=k;if(k==='double')R.page-=R.page%2;return rSync()}
  if(k==='rtl'){R.rtl=!R.rtl;return rSync()}if(k==='panel'){R.panel=!R.panel;return rSync()}
  if(['jp','zh','both'].includes(k)){R.view=k;return rSync()}
  if(k==='go')return translate();
  if(k==='export')return toast('正式版会另存为“'+MANGA[R.m].t+' [中文].cbz”，原文件不会被修改');
  if(k==='redo')return toast('正式版会带着前后文重新翻译这一句，并给出两三个备选');
});
$('#reader').addEventListener('input',e=>{if(e.target.dataset.k!=null){BB[+e.target.dataset.k].zh=e.target.value;rSync()}});
window.addEventListener('keydown',e=>{if(!$('#reader').classList.contains('open')||/INPUT|TEXTAREA|SELECT/.test(e.target.tagName))return;
  if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.stopPropagation();e.preventDefault();turn((e.key==='ArrowRight'?1:-1)*(R.rtl?-1:1))}
  else if(e.key==='Escape'){e.stopPropagation();$('#reader').classList.remove('open')}else if(e.key===' '){e.stopPropagation();e.preventDefault();turn(1)}},true);

/* ---------- 9. 自动获取：来源、AI 解析、搜索、下载、归类（全部为示例数据，不联网） ---------- */
const SRC=[['list','我的番剧订阅','订阅链接 · 每 30 分钟检查一次','正常'],['video','视频网站合集','通过 yt-dlp 支持 · 播放列表整份下载','正常'],['folder','家里的 NAS','WebDAV · \\\\nas\\media','正常']];
const sRender=()=>$('#srcs').innerHTML=SRC.map(s=>`<div class="src">${icon(s[0])}<b>${esc(s[1])}</b><span class="badge2 ok">${s[3]}</span><small>${esc(s[2])}</small></div>`).join('');
$('#sadd').onclick=async e=>{const u=$('#surl').value.trim();if(!u)return toast('先粘贴一个网站地址或订阅链接');const b=e.currentTarget;if(b.disabled)return;b.disabled=true;
  const host=u.replace(/^https?:\/\//,'').split('/')[0]||u,box=$('#gsteps');box.hidden=false;
  if(/rss|feed|\.xml/i.test(u)){box.innerHTML='<span class="st run">读取订阅内容</span>';await sleep(900);SRC.push(['list',host,'订阅链接 · 自动检查更新','正常'])}
  else{const st=['打开页面','分析页面结构','生成提取规则','试搜并验证'];box.innerHTML=st.map(x=>`<span class="st">${x}</span>`).join('');
    for(const el of $$('.st',box)){el.classList.add('run');await sleep(850);el.className='st ok'}
    SRC.push(['search',host,'AI 生成的规则 · 网站改版时自动重新生成','正常'])}
  sRender();$('#surl').value='';b.disabled=false;toast('已添加来源：'+host+'（演示，没有真的访问）')};
let gtype='anime';
$('#gtype').onclick=e=>{const b=e.target.closest('[data-t]');if(!b)return;gtype=b.dataset.t;$$('#gtype button').forEach(x=>x.classList.toggle('on',x===b));$('#gq').value=gtype==='anime'?'星屑邮局':'潮汐档案';$('#gres').innerHTML='<div class="empty">点“搜索”</div>';$('#gdl').innerHTML=''};
$('#gsearch').onclick=async()=>{const q=esc($('#gq').value.trim()||'星屑邮局'),box=$('#gres');$('#gdl').innerHTML='';box.innerHTML='<div class="empty">正在搜索 '+SRC.length+' 个来源，并用 AI 归并同一作品的不同写法…</div>';await sleep(1100);
  const R=gtype==='anime'?[[`${q} 第 1 季 · 全 12 集合集`,'1080p · HEVC 10bit · 内封简中 · 14.2 GB','来源：我的番剧订阅 · 做种 128',1,12],[`${q} 第 1 季 · 单集 01-12`,'1080p · H.264 · 外挂简繁 · 18.6 GB','来源：我的番剧订阅 · 12 个条目',0,12],[`${q} 第 1 季 · 全 12 集`,'720p · H.264 · 无字幕 · 5.1 GB','来源：视频网站合集',0,12]]
   :[[`${q}（专辑，10 首）`,'FLAC 24bit / 96kHz · 1.1 GB','来源：家里的 NAS',1,10],[`${q}（专辑，10 首）`,'MP3 320k · 96 MB','来源：视频网站合集',0,10]];
  box.innerHTML=`<div class="note" style="margin-bottom:10px">按你的偏好（${gtype==='anime'?'简中字幕、1080p、优先合集':'无损优先'}）排序，已合并重复条目。</div>`+R.map((r,i)=>`<div class="rs${r[3]?' on':''}" data-n="${r[4]}"><b>${r[0]}</b>${r[3]?'<span class="badge2 ai">AI 推荐</span>':'<span></span>'}<small>${r[1]}</small><small>${r[2]}</small><div class="acts"><button class="mp-btn" data-get="${i}">${icon('folder')} 下载并整理</button></div></div>`).join('')};
$('#gres').onclick=async e=>{const b=e.target.closest('[data-get]');if(!b||b.disabled)return;$$('#gres [data-get]').forEach(x=>x.disabled=true);
  const n=+b.closest('.rs').dataset.n,q=$('#gq').value.trim()||'星屑邮局',an=gtype==='anime',box=$('#gdl');
  const name=i=>an?`${q} - S01E${String(i+1).padStart(2,'0')}.mkv`:`${String(i+1).padStart(2,'0')} - ${['序','潮声','防波堤','雾港','灯塔','回信','夜航','退潮','码头','终章'][i%10]}.flac`;
  const dest=i=>an?`D:\\Media\\番剧\\${q}\\Season 01\\${name(i)}`:`D:\\Media\\音乐\\林间信号\\${q} (2024)\\${name(i)}`;
  box.innerHTML=`<h5 style="margin:16px 0 8px">下载队列 <span class="badge2" id="gstat">内置下载引擎 · 准备中</span></h5>`+[...Array(n).keys()].map(i=>`<div class="dl" style="--p:0"><span>${esc(name(i))}</span><small>等待中</small><div class="mp-pb"><i></i></div><code>→ ${esc(dest(i))}</code></div>`).join('');
  const rows=$$('.dl',box),p=rows.map(()=>0);let done=0;
  while(done<n){done=0;rows.forEach((r,i)=>{if(p[i]<1&&(i<4||p[i-4]>=1))p[i]=Math.min(1,p[i]+.02+Math.random()*.05);r.style.setProperty('--p',p[i]);
      if(p[i]>=1){done++;if(!r.classList.contains('done')){r.classList.add('done');$('small',r).textContent='已整理入库 ✓'}}else if(p[i]>0)$('small',r).textContent=Math.round(p[i]*100)+'% · '+(3+Math.random()*9).toFixed(1)+' MB/s'});
    $('#gstat').textContent=`内置下载引擎 · 已完成 ${done}/${n}`+(an?` · 连接 ${30+Math.random()*40|0} 个节点`:'');await sleep(110)}
  $('#gstat').className='badge2 ok';$('#gstat').textContent='全部完成，已按规则重命名并入库';
  box.insertAdjacentHTML('beforeend',`<div class="acts" style="margin-top:10px"><button class="mp-btn" data-nav="${an?'anime':'music'}">去${an?'番剧':'音乐'}库查看</button>${an?'<button class="mp-btn ghost" data-nav="aisub">缺字幕的交给 AI 字幕</button><span class="badge2">已订阅后续更新</span>':''}</div>`);
  $$('#gres [data-get]').forEach(x=>x.disabled=false);toast('下载完成，已自动整理到媒体库')};

/* ---------- 10. 小说去广告：AI 扫描、标记后找相似、清理与撤销（示例文本，广告内容均为虚构） ---------- */
const ADK={wm:['站点水印','以“本书首发”开头并带网址的整句','hi','在 212 章的章首和章尾重复出现'],url:['变形网址','用全角、空格、谐音字伪装的网址','hi','还原后是一个网址'],
 grp:['拉群推广','句子中间插入的加群、关注类推广','hi','和上下文不连贯'],junk:['防盗乱码','正文里随机插入的无意义字符','hi','不构成任何词语'],vote:['章末求票','章末的求票留言','mid','可能是作者留言，默认不勾选']};
const ad=(k,t)=>`<span class="ad" data-k="${k}">${t}</span>`,adp=(k,t)=>`<p class="ad adp" data-k="${k}">${t}</p>`;
MP.on('novel',o=>{
  const body=$('.mp-rbody',o),ps=$$('p',body).map(p=>p.textContent),cut=(s,r,x)=>{const i=Math.max(1,s.length*r|0);return s.slice(0,i)+x+s.slice(i)};
  ps[1]=cut(ps[1],.5,ad('grp','（看最新章节请加书友群 000000000）'));ps[3]=cut(ps[3],.6,ad('junk','＃＆＠％'));
  ps[4]=cut(ps[4],.45,ad('url','ｗ ｗ ｗ 点 某某书屋 点 ｅｘａｍｐｌｅ'));ps[6]=cut(ps[6],.7,ad('grp','【关注公号“某某书屋”，免费看全本】'));
  ps[8]=cut(ps[8],.5,ad('junk','＠＃＄％＾'));ps[10]=cut(ps[10],.4,ad('url','ｍ．某某书屋．ｅｘａｍｐｌｅ 手机阅读'));
  const P=ps.map(p=>`<p>${p}</p>`);
  P.splice(0,0,adp('wm','本书首发某某书屋 ｗｗｗ．某某书屋．ｅｘａｍｐｌｅ，请记住我们的网址！'));
  P.splice(7,0,adp('wm','本 书 首 发 某 某 书 屋，更新最快，无弹窗。'));
  P.push(adp('vote','求月票、求推荐票！作者码字不易，各位读者老爷多多支持。'),adp('wm','本书首发某某书屋 ｗｗｗ．某某书屋．ｅｘａｍｐｌｅ，手机用户请到 ｍ 站阅读。'));
  body.innerHTML='<h2>第三章　等信的人</h2>'+P.join('');
  const wrap=document.createElement('div');wrap.className='mp-rwrap';body.replaceWith(wrap);wrap.appendChild(body);
  wrap.insertAdjacentHTML('beforeend','<div class="mp-rside"><div></div></div>');const side=$('.mp-rside>div',o);
  $('[data-tts]',o).insertAdjacentHTML('afterend','<button data-ad>去广告</button>');
  let hits=[],rules=[],scanned=false,busy=false,un=0;const st={open:false,raw:false};
  const cls=()=>{o.classList.toggle('adopen',st.open);o.classList.toggle('showraw',st.raw)};
  $$('[data-th]',o).forEach(b=>b.addEventListener('click',cls));
  $('[data-ad]',o).onclick=()=>{st.open=!st.open;cls()};
  const txt=el=>el.textContent.replace(/\s+/g,' ').slice(0,26),add=(el,conf,why)=>{if(hits.some(h=>h.el===el))return;hits.push({el,conf,why,on:conf==='hi'});el.classList.add('found')};
  function render(){const live=hits.filter(h=>!h.el.classList.contains('gone')),gone=hits.length-live.length,sel=live.filter(h=>h.on).length;
    $('[data-ad]',o).textContent=gone?`去广告 · 已清理 ${gone}`:'去广告';
    side.innerHTML=`<h4>去广告</h4><p>可以让 AI 扫描全书，也可以在正文里选中一段广告，标记后让 AI 找出相似的内容。原文件不会被改动，随时可以撤销。</p>
     <button class="pri" data-a="scan"${busy?' disabled':''}>${scanned?'重新扫描':'AI 扫描全书'}</button>
     <div class="mp-adst" id="adst"></div>
     ${rules.length?'<h4 style="margin-top:8px">识别规则</h4>'+rules.map(r=>`<div class="mp-adrule"><b>命中 ${r[1]} 处</b>${esc(r[0])}</div>`).join(''):''}
     ${hits.length?`<h4 style="margin-top:14px">本章发现 ${hits.length} 处${scanned?' · 全书 1,284 处':''}</h4>`+hits.map((h,i)=>{const g=h.el.classList.contains('gone');return `<label class="mp-adhit${g?' gone':''}" data-h="${i}"><input type="checkbox"${h.on?' checked':''}${g?' disabled':''}><span>${esc(txt(h.el))}…<span class="mp-adtag ${h.conf}">${h.conf==='hi'?'把握高':'需确认'}</span></span><small>${esc(h.why)}</small></label>`}).join('')
       +`<div style="display:flex;gap:8px;flex-wrap:wrap;margin-top:10px"><button class="pri" data-a="clean"${sel?'':' disabled'}>清理选中的 ${sel} 处</button><button data-a="undo"${gone?'':' disabled'}>撤销</button><button data-a="raw"${gone?'':' disabled'}>${st.raw?'隐藏已清理内容':'对照原文'}</button></div>`
       +(gone?`<p style="margin-top:14px">已清理 ${gone} 处。现在只是阅读时隐藏，原文件没有改动。</p><div style="display:flex;gap:8px;flex-wrap:wrap"><button data-a="export">导出干净的副本</button><button data-a="replace">替换原文件</button></div><label style="display:flex;gap:8px;align-items:center;margin-top:12px;font-size:12.5px"><input type="checkbox" class="mp-sw" checked>同一来源的其他书自动套用这些规则</label>`:''):'<p style="margin-top:10px">还没有发现广告。试试点上面的按钮，或者在正文里选中一段文字。</p>'}`}
  side.onchange=e=>{const l=e.target.closest('[data-h]');if(l){hits[+l.dataset.h].on=e.target.checked;render()}};
  side.onmouseover=e=>{const l=e.target.closest('[data-h]');$$('.ad.flash',body).forEach(x=>x.classList.remove('flash'));if(l)hits[+l.dataset.h].el.classList.add('flash')};
  side.onclick=async e=>{const l=e.target.closest('[data-h]');if(l&&e.target.tagName!=='INPUT')hits[+l.dataset.h].el.scrollIntoView({behavior:'smooth',block:'center'});
    const b=e.target.closest('[data-a]');if(!b||b.disabled)return;const a=b.dataset.a;
    if(a==='scan'){busy=true;render();const box=$('#adst',side),names=['本地规则匹配','挑出可疑片段','AI 判断并归纳规则','应用到全书'];box.innerHTML=names.map(n=>`<span>${n}</span>`).join('');
      for(const el of $$('span',box)){el.className='run';await sleep(700);el.className='ok'}
      $$('.ad',body).forEach(el=>{const k=ADK[el.dataset.k];if(k)add(el,k[2],k[3])});
      rules=Object.keys(ADK).map(k=>[ADK[k][1],$$(`.ad[data-k="${k}"]`,body).length]).concat(rules.filter(r=>r[2]));
      scanned=true;busy=false;render();const box2=$('#adst',side);box2.innerHTML=names.map(n=>`<span class="ok">${n}</span>`).join('');
      return toast('扫描完成：只把 312 条可疑片段发给了 AI，没有上传整本书（演示）')}
    if(a==='clean'){hits.filter(h=>h.on).forEach(h=>h.el.classList.add('gone'));render();return toast('已清理。原文件没有改动，可以随时撤销')}
    if(a==='undo'){hits.forEach(h=>h.el.classList.remove('gone'));st.raw=false;cls();render();return toast('已撤销，恢复为原文')}
    if(a==='raw'){st.raw=!st.raw;cls();return render()}
    if(a==='export')return toast('已导出：雾港纪事（净化版）.epub，和原文件放在一起（演示）');
    if(a==='replace')return toast('已替换原文件，旧文件进了回收站（演示）')};
  /* 选中文字后的小菜单：标记为广告 / 标记并查找相似 */
  let pop=$('.mp-selpop');if(!pop){pop=document.createElement('div');pop.className='mp-selpop';pop.innerHTML='<button data-s="one">标记为广告</button><button data-s="sim">标记，并让 AI 找相似的清理全书</button>';document.body.appendChild(pop)}
  const showPop=(x,y)=>{pop.classList.add('open');pop.style.left=Math.max(8,Math.min(innerWidth-pop.offsetWidth-8,x))+'px';pop.style.top=Math.max(8,y-pop.offsetHeight-8)+'px'};
  const hasSel=()=>{const s=getSelection();return s&&!s.isCollapsed&&body.contains(s.anchorNode)&&s.toString().trim().length>1};
  body.addEventListener('mouseup',e=>setTimeout(()=>{if(hasSel())showPop(e.clientX-80,e.clientY-6);else pop.classList.remove('open')},0));
  body.addEventListener('contextmenu',e=>{if(hasSel()){e.preventDefault();showPop(e.clientX-80,e.clientY-6)}});
  body.addEventListener('scroll',()=>pop.classList.remove('open'));
  pop.onmousedown=e=>e.preventDefault();
  pop.onclick=e=>{const b=e.target.closest('[data-s]');if(!b||!hasSel()||!o.classList.contains('open'))return;const s=getSelection(),r=s.getRangeAt(0);pop.classList.remove('open');
    const node=n=>n.nodeType===1?n:n.parentElement;let el=node(r.startContainer).closest('.ad')||node(r.endContainer).closest('.ad'),known=!!el;
    if(!el){try{el=document.createElement('span');el.className='ad';el.dataset.k='u'+(++un);r.surroundContents(el)}catch(err){return toast('请在同一段里选择要标记的文字')}}
    s.removeAllRanges();st.open=true;cls();add(el,'hi','你手动标记的');hits.find(h=>h.el===el).on=true;
    if(b.dataset.s==='sim'){const sim=known?$$(`.ad[data-k="${el.dataset.k}"]`,body).filter(x=>x!==el):[];sim.forEach(x=>add(x,'hi','和你标记的内容相似'));
      rules.push([`你标记的样本：“${txt(el)}…”及其变体`,sim.length+1,1]);toast(sim.length?`找到 ${sim.length} 处相似内容，已列在右边，确认后清理`:'全书没有找到相似内容，只标记了这一处')}
    else toast('已标记。可以继续标记，或者让 AI 找相似的');
    render()};
  render();
});

/* ---------- 11. 每日推荐（示例数据：播放统计和推荐理由都是预先写好的规则生成） ---------- */
const DY={seed:3,mode:'local',list:[],fav:new Set(),busy:false};
const DB={sim:['b-sim','相似'],old:['b-old','旧爱'],nev:['b-nev','没听过'],exp:['b-exp','探索'],neo:['b-new','新歌 · 试听区'],kept:['b-nev','已入库']};
function dBuild(){let x=DY.seed*9301+49297;const rnd=()=>(x=(x*9301+49297)%233280)/233280,L=MP.songs.filter(s=>!s.trial&&s.trial!==0&&!DY.list.some(r=>r.b==='kept'&&r.s===s)),sh=a=>a.map(s=>[rnd(),s]).sort((a,b)=>a[0]-b[0]).map(v=>v[1]);
  const top=[...L].sort((a,b)=>b.plays-a.plays).slice(0,3),used=new Set(),take=(arr,n,b,why)=>sh(arr.filter(s=>!used.has(s.id))).slice(0,n).map(s=>(used.add(s.id),{s,b,why:why(s)}));
  DY.top=top;
  return [...take(L.filter(s=>top.some(t=>t.a===s.a&&t!==s)||Math.abs(s.bpm-top[0].bpm)<=8&&!top.includes(s)),3,'sim',s=>{const t=top.find(t=>t.a===s.a);return t?`和你最近循环的《${t.t}》是同一位艺人`:`节奏和《${top[0].t}》接近（${s.bpm} BPM）`}),
   ...take(L.filter(s=>s.plays>=20&&s.last>=90),2,'old',s=>`以前听过 ${s.plays} 次，已经 ${s.last>300?'快一年':s.last+' 天'}没听了`),
   ...take(L.filter(s=>s.plays<=3),2,'nev',s=>`入库 ${s.added} 天，${s.plays?'只听过 '+s.plays+' 次':'还没听过'}`),
   ...take(L,1,'exp',s=>`换换口味：${s.bpm>top[0].bpm?'比你平时听的更快':'比你平时听的更慢'}（${s.bpm} BPM）`)]}
function dRender(){const d=new Date(),neo=DY.mode==='new'?DY.list.filter(r=>r.b==='neo'||r.b==='kept'):[],all=[...neo,...DY.list.filter(r=>r.b!=='neo'&&r.b!=='kept')],t=DY.top;
  $('#ddate').textContent=`${d.getMonth()+1} 月 ${d.getDate()} 日 · 周${'日一二三四五六'[d.getDay()]}`;
  $('#dban').style.backgroundImage=coverCSS(all[0]?all[0].s.hue:210,1);$('#dmos').innerHTML=all.slice(0,4).map(r=>`<i style="background-image:${coverCSS(r.s.hue)}"></i>`).join('');
  $('#dsum').textContent=`昨天你听了 23 首，大多是 70 到 90 BPM 的慢速器乐，《${t[0].t}》循环了 6 次。今天多放了几首同类的，翻出两首很久没听的旧歌，再加一首平时不太听的类型。`;
  $('#dplay').innerHTML=`${icon('play')} 播放全部 · ${all.length} 首`;
  $('#dstats').innerHTML=[['23 首','昨日播放 · 1 小时 12 分'],[t[0].a,'最近 7 天最常听'],['70–90 BPM','偏好的节奏'],['深夜 22–24 点','最常听歌的时段'],['3 首','昨天被跳过，已降权']].map(x=>`<div><b>${esc(x[0])}</b>${x[1]}</div>`).join('');
  $('#dlist').innerHTML=all.map((r,i)=>{const s=r.s,dl=r.b==='neo'&&r.p<1;return `<div class="drow${!MP.state.file&&s.id===MP.state.idx?' on':''}" data-d="${s.id}" style="animation-delay:${i*40}ms${dl?';--p:'+r.p:''}"><span class="mp-cover" style="background-image:${coverCSS(s.hue)}"></span><span><b>${esc(s.t)}</b><small>${esc(s.a)} · ${s.fmt}</small></span><span class="why">${esc(r.why)}</span><span class="badge2 ${DB[r.b][0]}">${dl?'下载中 '+Math.round(r.p*100)+'%':DB[r.b][1]}</span><span class="ra">${r.b==='neo'&&!dl?`<button data-keep="${s.id}">留下</button>`:''}<button class="${DY.fav.has(s.id)?'fav':''}" data-like="${s.id}">喜欢</button><button data-no="${s.id}">不感兴趣</button></span>${dl?'<div class="mp-pb"><i></i></div>':''}</div>`}).join('')||'<div class="empty">今天的推荐都被你划掉了，点“换一批”。</div>';
  return all}
const dNew=()=>{DY.list=dBuild().concat(DY.list.filter(r=>r.b==='neo'||r.b==='kept'));dRender()};
$('#dshuf').onclick=()=>{DY.seed++;dNew();toast('换了一批（新歌保留）')};
$('#dplay').onclick=()=>{const a=dRender().filter(r=>!(r.b==='neo'&&r.p<1));if(a.length){MP.setTrack(a[0].s.id,true);toast(`正在播放「今日推荐」· ${a.length} 首`)}};
$('#dsave').onclick=()=>{const d=new Date(),name=`每日推荐 ${d.getMonth()+1}-${d.getDate()}`,ids=dRender().filter(r=>!r.s.trial).map(r=>r.s.id);PLS=PLS.filter(p=>p.name!==name);PLS.push({id:'p'+Date.now(),name,ids});savePl();mRender();toast(`已存为歌单「${name}」，在左侧歌单里`)};
$('#dlist').onclick=e=>{const row=e.target.closest('[data-d]');if(!row)return;const id=+row.dataset.d,r=DY.list.find(x=>x.s.id===id),b=e.target.closest('button');
  if(!b){if(r.b==='neo'&&r.p<1)return toast('还在下载，稍等一下');return MP.setTrack(id,true)}
  if(b.dataset.like){DY.fav.has(id)?DY.fav.delete(id):DY.fav.add(id);b.classList.toggle('fav');return toast(DY.fav.has(id)?'已记下：之后多推这类':'已取消')}
  if(b.dataset.keep){r.b='kept';r.s.trial=false;r.why='已移入正式库，并补全了标签和封面';dRender();mRender();return toast(`已留下：D:\\Media\\音乐\\${r.s.a}\\${r.s.al}\\${r.s.t}.flac（演示）`)}
  if(b.dataset.no){row.classList.add('out');setTimeout(()=>{DY.list=DY.list.filter(x=>x!==r);if(r.s.trial)r.s.trial=0;dRender()},300);return toast(r.b==='neo'?'已从试听区删除，之后少推这类':'已记下：之后少推这类')}};
$('#dmode').onclick=async e=>{const b=e.target.closest('[data-m]');if(!b||DY.busy||b.dataset.m===DY.mode)return;DY.mode=b.dataset.m;$$('#dmode button').forEach(x=>x.classList.toggle('on',x===b));
  const box=$('#dsteps'),note=$('#dnote');if(DY.mode==='local'){box.hidden=true;note.textContent='不联网也能用';note.className='badge2';return dRender()}
  if(DY.list.some(r=>r.b==='neo'||r.b==='kept')){box.hidden=false;return dRender()}
  DY.busy=true;box.hidden=false;const names=['分析口味','查找相似歌曲','核对是否真实存在','在我的来源里搜索','下载到试听区'];box.innerHTML=names.map(n=>`<span class="st">${n}</span>`).join('');const els=$$('.st',box);
  note.textContent='正在准备新歌…';note.className='badge2 ai';
  for(let i=0;i<4;i++){els[i].classList.add('run');await sleep(750);els[i].className='st ok'}
  els[4].classList.add('run');const t=DY.top;
  const N=[['退潮以后','林间信号','潮汐档案 II','FLAC 24/96',76,200,57,[0,5,3,4],74,`你常听${t[0].a==='林间信号'?'这位艺人':'林间信号'}，这张专辑库里还没有`],['末班渡轮','北纬三十度','站台以北 · B 面','FLAC',82,255,52,[0,3,5,4],92,'和《北站》风格相近（来自相似歌曲数据）'],['静电','Null Meridian','Cold Boot','Opus 160k',70,172,48,[0,4,2,5],120,'AI 提名：氛围接近你深夜常听的那几首']]
   .map(x=>{const s={t:x[0],a:x[1],al:x[2],fmt:x[3],dur:x[4],hue:x[5],root:x[6],prog:x[7],bpm:x[8],ly:[],lrc:[],id:MP.songs.length,plays:0,added:0,last:999,trial:true};MP.songs.push(s);return {s,b:'neo',why:x[9],p:0}});
  DY.list=N.concat(DY.list);dRender();
  while(N.some(r=>r.p<1)){N.forEach((r,i)=>{if(i===0||N[i-1].p>.4)r.p=Math.min(1,r.p+.04+Math.random()*.06)});if(document.body.dataset.view==='daily')dRender();await sleep(140)}
  els[4].className='st ok';note.textContent='找到 3 首，另有 2 首没找到，已放进“想听清单”';note.className='badge2 ok';DY.busy=false;dRender();toast('3 首新歌已下载到试听区，7 天内没点“留下”会自动清理')};

/* ---------- 12. 整理文件：识别、对照预览、执行、撤销（示例文件名，不读写任何文件） ---------- */
const OF=[
 ['v','D:\\下载\\[Hoshi-Sub] Hoshikuzu Yuubinkyoku - 03 [1080p][HEVC].mkv','番剧\\星屑邮局 (2024)\\Season 01\\星屑邮局 - S01E03.mkv','hi','文件名规则','字幕组命名，直接拆出作品名和集数'],
 ['v','D:\\下载\\[Hoshi-Sub] Hoshikuzu Yuubinkyoku - 03 [1080p][HEVC].sc.ass','番剧\\星屑邮局 (2024)\\Season 01\\星屑邮局 - S01E03.zh.ass','hi','配套文件','外挂字幕跟着视频走，改成对应的名字'],
 ['v','D:\\下载\\雾港异闻录 07v2.mkv','番剧\\雾港异闻录 (2025)\\Season 01\\雾港异闻录 - S01E07.mkv','hi','资料库','片名和集数与资料库吻合'],
 ['v','E:\\迅雷下载\\钢之候鸟.Hagane.no.Wataridori.2023.BD1080p.mkv','电影\\钢之候鸟 (2023)\\钢之候鸟 (2023).mkv','hi','资料库','时长 1 小时 52 分，判断为剧场版'],
 ['m','D:\\下载\\音乐\\01.flac','音乐\\林间信号\\潮汐档案 (2024)\\01 - 潮汐档案 · 序.flac','hi','声音指纹','文件名没有信息，靠声音认出来的；同时补全标签和封面'],
 ['m','D:\\下载\\track07 (1).mp3','音乐\\北纬三十度\\站台以北 (2022)\\07 - 北站.mp3','hi','声音指纹','同时补全标签和封面'],
 ['v','D:\\下载\\新建文件夹 (2)\\xxyj_s1_04_final(1).mp4','番剧\\星屑邮局 (2024)\\Season 01\\星屑邮局 - S01E04.mp4','mid','AI','AI 判断“xxyj”是“星屑邮局”的拼音首字母'],
 ['v','D:\\下载\\wgywl 特典 NCOP.mkv','番剧\\雾港异闻录 (2025)\\Specials\\雾港异闻录 - NCOP.mkv','mid','AI','无字幕片头，放进特典文件夹，不和正片混在一起'],
 ['v','E:\\迅雷下载\\Bento.Biyori.EP01.720p.mkv','重复待处理\\便当日和 - S01E01 (720p).mkv','mid','重复版本','库里已有 1080p 版本，这个先挪到“重复待处理”，不删除'],
 ['m','桌面\\lj_fbd.flac','音乐\\林间信号\\潮汐档案 (2024)\\09 - 防波堤.flac','mid','AI','声音指纹没有结果，AI 根据文件名和时长推测'],
 ['v','D:\\下载\\VID_20240812_193355.mp4','','lo','','看起来是手机拍摄的个人录像，建议不动'],
 ['m','D:\\下载\\录音 (3).m4a','','lo','','认不出是什么，保持原位']];
let oRows=[],oBusy=false,oType='all';
$('#otype').onclick=e=>{const b=e.target.closest('[data-t]');if(!b||oBusy)return;oType=b.dataset.t;$$('#otype button').forEach(x=>x.classList.toggle('on',x===b))};
const oSync=()=>{const n=$$('#olist input:checked').length,go=$('#oexec');if(go){go.textContent=`执行整理（${n} 项）`;go.disabled=!n}};
$('#ogo').onclick=async()=>{if(oBusy)return;oBusy=true;const box=$('#osteps'),stat=$('#ostat'),list=$('#olist'),acts=$('#oacts');acts.hidden=true;box.hidden=false;
  const names=['解析文件名','比对资料库与声音指纹','AI 识别剩下的','生成整理方案'];box.innerHTML=names.map(n=>`<span class="st">${n}</span>`).join('');list.innerHTML='<div class="empty">正在识别…</div>';stat.className='badge2 ai';
  oRows=OF.filter(f=>oType==='all'||(oType==='video'?f[0]==='v':f[0]==='m'));const els=$$('.st',box);
  for(let i=0;i<4;i++){els[i].classList.add('run');stat.textContent=names[i]+'…';await sleep(i===2?1100:700);els[i].className='st ok'}
  const G=[['hi','把握高 · 默认勾选'],['mid','需要你看一眼 · AI 推测或有重复'],['lo','认不出 · 保持原位，不会动']];
  list.innerHTML=G.map(g=>{const rs=oRows.filter(f=>f[3]===g[0]);return rs.length?`<div class="og">${g[1]} <span class="badge2">${rs.length}</span></div>`+rs.map(f=>{const i=oRows.indexOf(f);return `<label class="orow${f[2]?'':' skip'}" data-o="${i}"><input type="checkbox"${f[3]!=='lo'?' checked':' disabled'}><code class="from">${esc(f[1])}</code><span class="badge2 ${f[3]==='hi'?'ok':f[3]==='mid'?'no':''}">${f[4]||'未识别'}</span><code class="to">${f[2]?esc('D:\\Media\\'+f[2]):'保持原位'}</code><small>${esc(f[5])}</small></label>`}).join(''):''}).join('');
  const ai=oRows.filter(f=>f[4]==='AI').length;stat.className='badge2 ok';stat.textContent=`识别完成 · ${oRows.length} 个文件，其中 ${ai} 个交给了 AI`;
  acts.hidden=false;acts.innerHTML='<button class="mp-btn" id="oexec"></button><span class="badge2" id="omodeb"></span>';$('#omodeb').textContent='方式：'+$('#omode').value;oSync();oBusy=false};
$('#omode').onchange=()=>{const b=$('#omodeb');if(b)b.textContent='方式：'+$('#omode').value};
$('#olist').onchange=oSync;
$('#oacts').onclick=async e=>{const b=e.target.closest('button');if(!b||oBusy)return;const stat=$('#ostat'),acts=$('#oacts'),mode=$('#omode').value;
  if(b.id==='oexec'){oBusy=true;const rows=$$('#olist .orow').filter(r=>$('input',r).checked);$$('#olist input').forEach(i=>i.disabled=true);b.disabled=true;stat.className='badge2 ai';
    for(let i=0;i<rows.length;i++){rows[i].classList.add('done');stat.textContent=`正在${mode==='仅在媒体库中整理'?'整理':mode}… ${i+1}/${rows.length}`;await sleep(260)}
    stat.className='badge2 ok';stat.textContent=`完成 · 已整理 ${rows.length} 个文件，没有删除任何文件`;
    acts.innerHTML=`<button class="mp-btn ghost" id="oundo">撤销这一批</button><button class="mp-btn ghost" data-nav="anime">去番剧库查看</button><span class="badge2">${mode==='硬链接'?'原文件保留，仍可做种':mode==='复制'?'原文件保留':mode==='移动'?'播放进度、歌单、字幕关联都已保留':'硬盘上的文件没有动'}</span>`;oBusy=false;return toast('整理完成，媒体库已更新（演示，没有真的移动文件）')}
  if(b.id==='oundo'){$$('#olist .orow.done').forEach(r=>r.classList.remove('done'));$$('#olist .orow:not(.skip) input').forEach(i=>i.disabled=false);stat.className='badge2';stat.textContent='已撤销 · 所有文件回到原来的位置和名字';
    acts.innerHTML='<button class="mp-btn" id="oexec"></button><span class="badge2" id="omodeb"></span>';$('#omodeb').textContent='方式：'+mode;oSync();return toast('已撤销这一批')}};

/* ---------- 初始化 ---------- */
MP.on('ready',()=>{mRender();aRender();cRender();aiNote();sRender();DY.list=dBuild();dRender()});
MP.on('track',()=>{const v=document.body.dataset.view;if(v==='music')mRender();if(v==='daily'&&!DY.busy)dRender()});
MP.on('view',v=>{if(v==='music')mRender();if(v==='daily')dRender();pop.classList.remove('open')});
})();
