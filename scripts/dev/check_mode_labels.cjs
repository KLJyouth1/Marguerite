// 双入口分析：
// 入口A = ChatComposerPlanMode（composer 上的 Plan 徽章开关）→ plans.setMode → collaboration_mode
// 入口B = ChatComposerModelRouting（composer 齿轮弹层）→ models.routing.set → mode.switch
// 前者控制「计划模式」（只读探索），后者控制「模型路由/编排形态」（router/ensemble/bloom/omni）
// 用户困惑点：两者都叫"模式"且都在 composer 上，语义不同但 UI 未区分
const fs = require('fs');
const zh = JSON.parse(fs.readFileSync('D:/Downloads/Marguerite/webui/src/locales/zh-Hans.json', 'utf8'));
const mr = zh.chat?.composer || {};
console.log('modelRoutingOff:', mr.modelRoutingOff);
console.log('modelRoutingSquillaRouter:', mr.modelRoutingSquillaRouter);
console.log('modelRoutingEnsemble:', mr.modelRoutingEnsemble);
console.log('modelRoutingBloomEnsemble:', mr.modelRoutingBloomEnsemble);
console.log('modelRoutingOmni:', mr.modelRoutingOmni);
console.log('planMode:', JSON.stringify(zh.chat?.planMode));
