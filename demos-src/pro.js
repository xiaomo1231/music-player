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
$('#msort').insertAdjacentHTML('beforebegin','<select class="sel" id="mgrp"><option value="none">不分组</option><option value="a">按艺人分组</option><option value="al">按专辑分组</option><option value="fmt">按格式分组</option></select>');
function mFiltered(){
  const sm=SMART.find(x=>x[0]===M.f);let l=MP.songs.filter(s=>(!sm||sm[3](s))&&(!M.q||(s.t+s.a+s.al+s.fmt).toLowerCase().includes(M.q)));
  if(M.sort==='plays')l.sort((a,b)=>b.plays-a.plays);else if(M.sort==='added')l.sort((a,b)=>a.added-b.added);else if(M.sort==='title')l.sort((a,b)=>a.t.localeCompare(b.t,'zh'));
  else if(M.f==='hot')l.sort((a,b)=>b.plays-a.plays);else if(M.f==='new')l.sort((a,b)=>a.added-b.added);
  return l;
}
function mRender(){
  $('#smart').innerHTML=SMART.map(x=>{const n=MP.songs.filter(x[3]).length;return `<div class="sc${M.f===x[0]?' on':''}" data-f="${x[0]}" style="background-image:${coverCSS(x[4],1)}"><b>${x[1]}</b><span>${n} 首 · ${x[2]}</span><button data-play="${x[0]}" title="立即播放">${icon('play')}</button></div>`}).join('');
  $('#mgroup').innerHTML=[['all','全部歌曲']].concat(SMART).map(x=>`<button class="mp-chip${M.f===x[0]?' on':''}" data-f="${x[0]}">${x[1]}</button>`).join('')+(M.q?`<span class="badge2 ai">搜索：${esc(M.q)}</span>`:'');
  const l=mFiltered();$('#mcount').textContent=`共 ${MP.songs.length} 首（演示数据），当前显示 ${l.length} 首 · 点卡片上的播放键可以直接播放整组`;
  const row=(s,n)=>`<div class="mp-row mrow${!MP.state.file&&s.id===MP.state.idx?' on':''}" data-i="${s.id}"><span class="mp-n">${String(n).padStart(2,'0')}</span><span class="mp-cover" style="background-image:${coverCSS(s.hue)}"></span><span class="mp-t">${s.t}<small>${s.a}</small></span><span class="mp-al">${s.al}</span><span class="mp-fmt">${s.fmt}</span><span class="pc">${s.plays?s.plays+' 次':'未播放'}</span><span class="mp-d">${fmtT(s.dur)}</span><span class="ra"><button class="${FAV.has(s.id)?'fav':''}" data-fav="${s.id}" title="收藏">${icon('heart')}</button><button data-cv="${s.id}" title="转换格式">${icon('convert')}</button></span></div>`;
  let h='';
  if(!l.length)h='<div class="empty">没有符合条件的歌曲</div>';
  else if(M.g==='none')h=l.map((s,i)=>row(s,i+1)).join('');
  else{const key=s=>M.g==='fmt'?s.fmt.split(' ')[0]:s[M.g],gs={};l.forEach(s=>(gs[key(s)]=gs[key(s)]||[]).push(s));
    h=Object.keys(gs).sort((a,b)=>a.localeCompare(b,'zh')).map(k=>`<div class="grp">${k} · ${gs[k].length} 首 <button class="mp-chip" data-gplay="${esc(k)}">${icon('play')} 播放</button></div>`+gs[k].map((s,i)=>row(s,i+1)).join('')).join('')}
  $('#mlist').innerHTML=h;
}
function playSet(list,name){if(!list.length)return toast('这个分组里还没有歌曲');MP.setTrack(list[0].id,true);toast(`正在播放「${name}」· ${list.length} 首`)}
$('[data-view="music"]').addEventListener('click',e=>{
  const b=e.target.closest('[data-play],[data-f],[data-fav],[data-cv],[data-gplay]');if(!b)return;
  if(b.dataset.play){e.stopPropagation();const x=SMART.find(x=>x[0]===b.dataset.play);const keep=M.f;M.f=x[0];const l=mFiltered();M.f=keep;return playSet(l,x[1])}
  if(b.dataset.gplay!=null){e.stopPropagation();const k=b.dataset.gplay;return playSet(mFiltered().filter(s=>(M.g==='fmt'?s.fmt.split(' ')[0]:s[M.g])===k),k)}
  if(b.dataset.fav){e.stopPropagation();const i=+b.dataset.fav;FAV.has(i)?FAV.delete(i):FAV.add(i);toast(FAV.has(i)?'已加入收藏':'已取消收藏');return mRender()}
  if(b.dataset.cv){e.stopPropagation();return quickConvert(MP.songs[+b.dataset.cv])}
  if(b.dataset.f){M.f=b.dataset.f;mRender()}
});
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

/* ---------- 初始化 ---------- */
MP.on('ready',()=>{mRender();aRender();cRender();aiNote()});
MP.on('track',()=>{if(document.body.dataset.view==='music')mRender()});
MP.on('view',v=>{if(v==='music')mRender();pop.classList.remove('open')});
})();
