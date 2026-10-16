#!/usr/bin/env node
// 向 6 个语言文件注入 emptyStep1/2/3 键（插在 configuredEmpty 之后）
const fs = require('fs');
const path = require('path');

const translations = {
  'zh-Hans': {
    step1: '点击右上角「{add}」选择服务商',
    step2: '填入 API Key 并完成连通性验证',
    step3: '到「模型路由」选择默认模型与编排策略',
  },
  en: {
    step1: "Click '{add}' in the top-right corner to pick a provider",
    step2: 'Fill in the API key and verify connectivity',
    step3: "Choose the default model and orchestration strategy under 'Model routing'",
  },
  ja: {
    step1: '右上の「{add}」からプロバイダーを選択',
    step2: 'API キーを入力して接続を検証',
    step3: '「モデルルーティング」で既定モデルと戦略を選択',
  },
  de: {
    step1: "Oben rechts '{add}' wählen, um einen Anbieter auszuwählen",
    step2: 'API-Schlüssel eintragen und Verbindung prüfen',
    step3: "Unter 'Modell-Routing' Standardmodell und Strategie wählen",
  },
  es: {
    step1: "Pulsa '{add}' arriba a la derecha para elegir un proveedor",
    step2: 'Introduce la clave API y verifica la conectividad',
    step3: "En 'Enrutamiento de modelos' elige modelo y estrategia",
  },
  fr: {
    step1: "Cliquez sur « {add} » en haut à droite pour choisir un fournisseur",
    step2: 'Saisissez la clé API et vérifiez la connectivité',
    step3: "Dans « Routage des modèles », choisissez modèle et stratégie",
  },
};

let changed = 0;
for (const [locale, t] of Object.entries(translations)) {
  const file = path.join(__dirname, '..', 'src', 'locales', `${locale}.json`);
  const raw = fs.readFileSync(file, 'utf8');
  // 在 configuredEmpty 行后插入三键
  const anchor = /(\s*)"configuredEmpty":\s*"(?:[^"\\]|\\.)*",/;
  if (!anchor.test(raw)) { console.error(`${locale}: 未找到锚点`); continue; }
  if (raw.includes('"emptyStep1"')) { console.log(`${locale}: 已存在，跳过`); continue; }
  const esc = s => s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  const insert = `\n      "emptyStep1": "${esc(t.step1)}",\n      "emptyStep2": "${esc(t.step2)}",\n      "emptyStep3": "${esc(t.step3)}",`;
  fs.writeFileSync(file, raw.replace(anchor, (m) => m + insert), 'utf8');
  changed++;
  console.log(`${locale}: 注入完成`);
}
console.log(`共更新 ${changed} 个文件`);
