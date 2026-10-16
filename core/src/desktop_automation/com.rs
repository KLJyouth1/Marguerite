//! WPS 演示驱动：经 Windows COM 调 `Kwpp.Application` 生成可编辑 .pptx。
//!
//! 仅 Windows 编译（依赖 `windows` crate 的 COM/OLE 能力）。非 Windows 下本模块
//! 不编译（注册表也不会注册 WpsPpt），`desktop.launch` 返回 2401 未知应用。
//!
//! 流程：CoCreateInstance(Kwpp.Application) -> Visible=true -> Presentations.Add() ->
//! 逐页 Slides.Add + Shapes.AddTextbox 写入标题/要点 -> SaveAs(.pptx) ->
//! [`crate::tools::file_change::record_artifact_with_session`] 登记为产物（deck）。
//! 每步经 [`crate::desktop_automation::recorder::Recorder`] 落审计快照。

#![cfg(windows)]

use std::path::PathBuf;

use anyhow::{anyhow, Result as AnyResult};
use serde_json::{json, Value};

use windows::core::BSTR;
use windows::core::GUID;
use windows::core::PCWSTR;
use windows::Win32::Globalization::LOCALE_USER_DEFAULT;
use windows::Win32::System::Com::CLSIDFromProgID;
use windows::Win32::System::Com::CoCreateInstance;
use windows::Win32::System::Com::CoInitializeEx;
use windows::Win32::System::Com::IDispatch;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Com::COINIT_APARTMENTTHREADED;
use windows::Win32::System::Com::DISPATCH_METHOD;
use windows::Win32::System::Com::DISPATCH_PROPERTYGET;
use windows::Win32::System::Com::DISPATCH_PROPERTYPUT;
use windows::Win32::System::Com::DISPPARAMS;
use windows::Win32::System::Variant::VARIANT;

use crate::desktop_automation::driver::*;
use crate::desktop_automation::recorder::Recorder;
use crate::tools::file_change;

const DISPID_PROPERTYPUT: i32 = -3;

/// 对 IDispatch 的薄封装：属性读写 + 方法调用（COM 自动化 late-bound）。
struct Dispatch(IDispatch);

impl Dispatch {
    fn from_progid(prog_id: &str) -> AnyResult<Self> {
        unsafe {
            let bstr = BSTR::from(prog_id);
            let clsid = CLSIDFromProgID(PCWSTR::from_raw(bstr.as_ptr()))?;
            let disp: IDispatch = CoCreateInstance(&clsid, None, CLSCTX_ALL)?;
            Ok(Dispatch(disp))
        }
    }

    fn from_dispatch(d: IDispatch) -> Self {
        Dispatch(d)
    }

    fn get_id(&self, name: &str) -> AnyResult<i32> {
        unsafe {
            let bstr = BSTR::from(name);
            let name_pcwstr = PCWSTR::from_raw(bstr.as_ptr());
            let mut disp_id: i32 = 0;
            self.0.GetIDsOfNames(
                std::ptr::null::<GUID>(),
                &name_pcwstr as *const PCWSTR,
                1,
                LOCALE_USER_DEFAULT,
                &mut disp_id,
            )?;
            Ok(disp_id)
        }
    }

    fn get(&self, name: &str) -> AnyResult<VARIANT> {
        let id = self.get_id(name)?;
        unsafe {
            let mut result = VARIANT::default();
            let mut params = DISPPARAMS {
                rgvarg: std::ptr::null_mut(),
                rgdispidNamedArgs: std::ptr::null_mut(),
                cArgs: 0,
                cNamedArgs: 0,
            };
            self.0.Invoke(
                id,
                std::ptr::null::<GUID>(),
                LOCALE_USER_DEFAULT,
                DISPATCH_PROPERTYGET,
                &mut params as *mut DISPPARAMS,
                Some(&mut result as *mut VARIANT),
                Some(std::ptr::null_mut()),
                Some(std::ptr::null_mut::<u32>()),
            )?;
            Ok(result)
        }
    }

    fn put(&self, name: &str, value: &VARIANT) -> AnyResult<()> {
        let id = self.get_id(name)?;
        unsafe {
            let mut args = vec![value.clone()];
            let mut dispid_put = DISPID_PROPERTYPUT;
            let mut params = DISPPARAMS {
                rgvarg: args.as_mut_ptr(),
                rgdispidNamedArgs: &mut dispid_put as *mut i32,
                cArgs: 1,
                cNamedArgs: 1,
            };
            self.0.Invoke(
                id,
                std::ptr::null::<GUID>(),
                LOCALE_USER_DEFAULT,
                DISPATCH_PROPERTYPUT,
                &mut params as *mut DISPPARAMS,
                Some(std::ptr::null_mut()),
                Some(std::ptr::null_mut()),
                Some(std::ptr::null_mut::<u32>()),
            )?;
            Ok(())
        }
    }

    fn call(&self, name: &str, args: &[VARIANT]) -> AnyResult<VARIANT> {
        let id = self.get_id(name)?;
        unsafe {
            let mut args_owned: Vec<VARIANT> = args.to_vec();
            // COM 要求参数逆序入栈（最后一个形参在 rgvarg[0]）
            args_owned.reverse();
            let mut params = DISPPARAMS {
                rgvarg: args_owned.as_mut_ptr(),
                rgdispidNamedArgs: std::ptr::null_mut(),
                cArgs: args_owned.len() as u32,
                cNamedArgs: 0,
            };
            let mut result = VARIANT::default();
            self.0.Invoke(
                id,
                std::ptr::null::<GUID>(),
                LOCALE_USER_DEFAULT,
                DISPATCH_METHOD,
                &mut params as *mut DISPPARAMS,
                Some(&mut result as *mut VARIANT),
                Some(std::ptr::null_mut()),
                Some(std::ptr::null_mut::<u32>()),
            )?;
            Ok(result)
        }
    }

    fn call_dispatch(&self, name: &str, args: &[VARIANT]) -> AnyResult<Dispatch> {
        let v = self.call(name, args)?;
        let disp: IDispatch =
            IDispatch::try_from(&v).map_err(|e| anyhow!("无法从返回值提取 IDispatch: {e}"))?;
        Ok(Dispatch::from_dispatch(disp))
    }

    fn get_dispatch(&self, name: &str) -> AnyResult<Dispatch> {
        let v = self.get(name)?;
        let disp: IDispatch =
            IDispatch::try_from(&v).map_err(|e| anyhow!("无法从属性提取 IDispatch: {e}"))?;
        Ok(Dispatch::from_dispatch(disp))
    }
}

fn vstr(s: &str) -> VARIANT {
    VARIANT::from(BSTR::from(s))
}
fn vi32(n: i32) -> VARIANT {
    VARIANT::from(n)
}
fn vbool(b: bool) -> VARIANT {
    VARIANT::from(b)
}

fn com_err(e: impl std::fmt::Display) -> DesktopError {
    DesktopError::ComError(format!("COM 操作失败: {e}"))
}

pub struct ComDriver {
    prog_id: String,
}

impl ComDriver {
    pub fn new(prog_id: &str) -> Self {
        ComDriver {
            prog_id: prog_id.to_string(),
        }
    }

    /// 由大纲（标题 + 要点）生成 PPT 并保存为 .pptx。
    fn generate_presentation(
        &self,
        h: &AppHandle,
        outline: &Value,
        output_path: &str,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        let app = Dispatch::from_progid(&self.prog_id).map_err(|e| {
            DesktopError::ComError(format!(
                "无法创建 WPS 应用对象（Kwpp.Application）：请确认已安装 WPS 并注册 COM（以管理员运行 `wps /regserver`）。{e}"
            ))
        })?;

        Recorder::record_step(
            session,
            &UiAction {
                kind: "com_create".into(),
                params: json!({ "prog_id": self.prog_id }),
            },
            None,
            None,
            "CoCreateInstance",
        );

        // 可见（否则 SaveAs 可能无窗口交互异常）
        let _ = app.put("Visible", &vbool(true));

        let presentations = app.get_dispatch("Presentations").map_err(com_err)?;
        Recorder::record_step(
            session,
            &UiAction {
                kind: "get_presentations".into(),
                params: Value::Null,
            },
            None,
            None,
            "Presentations",
        );

        let pres = presentations.call_dispatch("Add", &[]).map_err(com_err)?;
        Recorder::record_step(
            session,
            &UiAction {
                kind: "presentation_add".into(),
                params: Value::Null,
            },
            None,
            None,
            "Presentations.Add",
        );

        // 解析大纲：支持 slides:[{title,bullets}] 或 bullets:[..] 单页
        let title = outline
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();
        let slide_specs: Vec<(String, Vec<String>)> =
            if let Some(arr) = outline.get("slides").and_then(|v| v.as_array()) {
                arr.iter()
                    .map(|s| {
                        let t = s
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let b = s
                            .get("bullets")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (t, b)
                    })
                    .collect()
            } else if let Some(arr) = outline.get("bullets").and_then(|v| v.as_array()) {
                let b: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect();
                vec![(title.clone(), b)]
            } else {
                vec![(title.clone(), vec![])]
            };

        let slides = pres.get_dispatch("Slides").map_err(com_err)?;
        for (i, (st, bullets)) in slide_specs.iter().enumerate() {
            // v3.3.1：逐页进度事件（artifact_progress）——前端实时渲染
            // 「正在生成第 i/N 页」；网关总线未注册时 no-op。
            if let Some(bus) = crate::events::global_bus() {
                let total = slide_specs.len();
                bus.publish(crate::events::Envelope::new(
                    crate::events::envelope::EventType::Progress,
                    json!({
                        "kind": "artifact_progress",
                        "artifact": "deck",
                        "path": output_path,
                        "step": i + 1,
                        "stepsTotal": total,
                        "percent": ((i as f64) / (total.max(1) as f64) * 100.0) as u32,
                        "detail": format!("生成第 {}/{} 页：{}", i + 1, total, st),
                    }),
                    session,
                ));
            }
            // 版式：首页标题版式(1)，其余标题+内容版式(2)
            let layout = vi32(if i == 0 { 1i32 } else { 2i32 });
            let index = vi32((i as i32) + 1);
            let slide = slides
                .call_dispatch("Add", &[index, layout])
                .map_err(com_err)?;

            // 标题
            let title_shape = slide.get_dispatch("Shapes").map_err(com_err)?;
            let title_text = title_shape
                .get_dispatch("Title")
                .and_then(|t| t.get_dispatch("TextFrame"))
                .and_then(|tf| tf.get_dispatch("TextRange"))
                .map_err(com_err)?;
            title_text.put("Text", &vstr(st)).map_err(com_err)?;

            // 要点：用文本框（msoTextOrientationHorizontal=1）写入，避免 Placeholder 索引依赖
            if !bullets.is_empty() {
                let shapes = slide.get_dispatch("Shapes").map_err(com_err)?;
                let tb = shapes
                    .call_dispatch(
                        "AddTextbox",
                        &[vi32(1), vi32(50), vi32(120), vi32(600), vi32(400)],
                    )
                    .map_err(com_err)?;
                let tf = tb.get_dispatch("TextFrame").map_err(com_err)?;
                let tr = tf.get_dispatch("TextRange").map_err(com_err)?;
                let text = bullets.join("\n");
                tr.put("Text", &vstr(&text)).map_err(com_err)?;
            }

            Recorder::record_step(
                session,
                &UiAction {
                    kind: "add_slide".into(),
                    params: json!({ "title": st, "bullets": bullets }),
                },
                None,
                None,
                "Slides.Add",
            );
        }

        // 保存为 .pptx（FileFormat 24 = OpenXML Presentation）
        let out_v = vstr(output_path);
        let fmt = vi32(24);
        pres.call("SaveAs", &[out_v, fmt]).map_err(com_err)?;
        Recorder::record_step(
            session,
            &UiAction {
                kind: "save_as".into(),
                params: json!({ "path": output_path }),
            },
            None,
            None,
            "Presentation.SaveAs",
        );

        let _ = pres.call("Close", &[]);
        let _ = app.call("Quit", &[]);

        // 登记为产物（deck）
        file_change::record_artifact_with_session(session, output_path, "deck");

        let snap = Recorder::record_step(
            session,
            &UiAction {
                kind: "generate_presentation".into(),
                params: json!({ "path": output_path }),
            },
            None,
            None,
            "done",
        );
        Ok(ActionReceipt::ok(
            &h.id,
            "generate_presentation",
            json!({ "path": output_path }),
            Some(snap),
        ))
    }
}

impl DesktopAppDriver for ComDriver {
    fn kind(&self) -> AppKind {
        AppKind::WpsPpt
    }

    fn launch(&self, _args: &Value) -> Result<AppHandle, DesktopError> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        match Dispatch::from_progid(&self.prog_id) {
            Ok(app) => {
                let _ = app.put("Visible", &vbool(true));
                Ok(AppHandle::new(AppKind::WpsPpt))
            }
            Err(e) => Err(DesktopError::ComError(format!(
                "无法启动 WPS（Kwpp.Application）：请确认已安装 WPS 并注册 COM（以管理员运行 `wps /regserver`）。{e}"
            ))),
        }
    }

    fn activate(&self, _h: &AppHandle) -> Result<(), DesktopError> {
        // MVP：WPS 实例在 generate_presentation 内自管理生命周期，activate 为 no-op。
        Ok(())
    }

    fn run_action(
        &self,
        h: &AppHandle,
        action: &UiAction,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError> {
        match action.kind.as_str() {
            "generate_presentation" => {
                let output = action
                    .param_str("output_path")
                    .or_else(|| action.param_str("path"))
                    .unwrap_or_else(|| {
                        let dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
                        dir.join(".marguerite")
                            .join("generated.pptx")
                            .to_string_lossy()
                            .to_string()
                    });
                let outline = action
                    .params
                    .get("outline")
                    .cloned()
                    .unwrap_or_else(|| action.params.clone());
                self.generate_presentation(h, &outline, &output, session)
            }
            other => Err(DesktopError::UnsupportedAction(other.to_string())),
        }
    }

    fn quit(&self, _h: &AppHandle) -> Result<(), DesktopError> {
        // MVP：WPS 实例在 generate_presentation 内已 Quit；此处 best-effort no-op。
        Ok(())
    }
}
