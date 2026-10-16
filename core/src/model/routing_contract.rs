//! Stable, deterministic routing adapter contract; no provider calls are made.
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, fmt};
const REDACTED: &str = "[REDACTED]";

#[derive(Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum RouteMode { Direct, Router, Ensemble, Bloom, #[default]
Omni }
impl RouteMode {
    pub fn as_str(self) -> &'static str { match self { Self::Direct=>"direct", Self::Router=>"router", Self::Ensemble=>"ensemble", Self::Bloom=>"bloom", Self::Omni=>"omni" } }
    pub fn parse(value: &str) -> Result<Self, RouteContractError> { match value { "direct"=>Ok(Self::Direct), "router"=>Ok(Self::Router), "ensemble"=>Ok(Self::Ensemble), "bloom"=>Ok(Self::Bloom), "omni"=>Ok(Self::Omni), _=>Err(RouteContractError::InvalidMode(value.to_owned())) } }
}
impl fmt::Debug for RouteMode { fn fmt(&self, f:&mut fmt::Formatter<'_>)->fmt::Result { f.write_str(self.as_str()) } }
impl Serialize for RouteMode { fn serialize<S>(&self, s:S)->Result<S::Ok,S::Error> where S:Serializer { s.serialize_str(self.as_str()) } }
impl<'de> Deserialize<'de> for RouteMode { fn deserialize<D>(d:D)->Result<Self,D::Error> where D:Deserializer<'de> { Self::parse(&String::deserialize(d)?).map_err(de::Error::custom) } }

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct RouteWeights {
    #[serde(default="default_capability_weight")] pub capability:f64,
    #[serde(default="default_cost_weight")] pub cost:f64,
    #[serde(default="default_latency_weight")] pub latency:f64,
}
fn default_capability_weight()->f64 { 0.5 } fn default_cost_weight()->f64 { 0.3 } fn default_latency_weight()->f64 { 0.2 }
impl Default for RouteWeights { fn default()->Self { Self { capability:0.5, cost:0.3, latency:0.2 } } }
impl RouteWeights { pub fn validate(&self)->Result<(),RouteContractError> { let v=[self.capability,self.cost,self.latency]; if v.iter().any(|x|!x.is_finite()||*x<0.)||v.iter().sum::<f64>()<=0. { Err(RouteContractError::InvalidWeights) } else { Ok(()) } } }

#[derive(Clone,Serialize)]
pub struct RouteRequest {
    pub request_id:String, pub intent:String, pub complexity:f64,
    #[serde(default)] pub mode:RouteMode, #[serde(default)] pub candidates:Vec<String>, #[serde(default)] pub weights:RouteWeights,
    #[serde(serialize_with="serialize_redacted_metadata")] pub metadata:BTreeMap<String,String>,
}
impl fmt::Debug for RouteRequest { fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result { f.debug_struct("RouteRequest").field("request_id",&self.request_id).field("intent",&self.intent).field("complexity",&self.complexity).field("mode",&self.mode).field("candidates",&self.candidates).field("weights",&self.weights).field("metadata",&redacted_metadata(&self.metadata)).finish() } }
impl<'de> Deserialize<'de> for RouteRequest {
    fn deserialize<D>(d:D)->Result<Self,D::Error> where D:Deserializer<'de> {
        #[derive(Deserialize)] struct Raw { request_id:String, intent:String, #[serde(default)] complexity:f64, #[serde(default)] mode:RouteMode, #[serde(default)] candidates:Vec<String>, #[serde(default)] weights:RouteWeights, #[serde(default)] metadata:BTreeMap<String,String> }
        let r=Raw::deserialize(d)?; Ok(Self { request_id:r.request_id,intent:r.intent,complexity:r.complexity,mode:r.mode,candidates:r.candidates,weights:r.weights,metadata:r.metadata })
    }
}
impl RouteRequest {
    #[allow(clippy::possible_missing_else)]
    pub fn validate(&self)->Result<(),Vec<RouteContractError>> { let mut e=Vec::new(); if self.request_id.trim().is_empty(){e.push(RouteContractError::InvalidRequestId);} if self.intent.trim().is_empty(){e.push(RouteContractError::InvalidIntent);} if !self.complexity.is_finite()||!(0.0..=1.0).contains(&self.complexity){e.push(RouteContractError::InvalidComplexity);} if self.candidates.iter().any(|c|c.trim().is_empty()){e.push(RouteContractError::InvalidCandidate);} if let Err(x)=self.weights.validate(){e.push(x);} if e.is_empty(){Ok(())}else{Err(e)} }
    pub fn normalize(mut self)->Result<Self,Vec<RouteContractError>> { self.request_id=self.request_id.trim().into(); self.intent=self.intent.trim().into(); self.candidates=self.candidates.into_iter().map(|c|c.trim().into()).collect(); self.validate().map(|_|self) }
}

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct CandidateProfile { pub model:String, pub capability:f64, pub cost:f64, pub latency:f64 }
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct CandidateScore { pub model:String, pub capability:f64, pub cost:f64, pub latency:f64, pub total:f64 }
#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct RouteDecision { pub request_id:String, pub mode:RouteMode, pub selected_model:Option<String>, pub candidates_considered:Vec<String>, pub scores:Vec<CandidateScore>, pub fallback:bool, pub explanation:String }

#[derive(Debug,Clone,PartialEq,Eq)]
pub enum RouteContractError { InvalidMode(String), InvalidRequestId, InvalidIntent, InvalidComplexity, InvalidCandidate, InvalidWeights }
impl RouteContractError { pub fn code(&self)->&'static str { match self { Self::InvalidMode(_)=>"invalid_route_mode",Self::InvalidRequestId=>"invalid_request_id",Self::InvalidIntent=>"invalid_intent",Self::InvalidComplexity=>"invalid_complexity",Self::InvalidCandidate=>"invalid_candidate",Self::InvalidWeights=>"invalid_weights" } } }
impl fmt::Display for RouteContractError { fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result { match self { Self::InvalidMode(v)=>write!(f,"invalid route mode: {v}"), _=>f.write_str(self.code()) } } }
impl std::error::Error for RouteContractError {}

pub fn score_candidate(p:&CandidateProfile,w:&RouteWeights)->CandidateScore { CandidateScore { model:p.model.clone(), capability:p.capability,cost:p.cost,latency:p.latency,total:p.capability*w.capability+(1.-p.cost)*w.cost+(1.-p.latency)*w.latency } }
pub fn decide(r:&RouteRequest,p:&[CandidateProfile])->Result<RouteDecision,Vec<RouteContractError>> { r.validate()?; let mut scores:Vec<_>=p.iter().filter(|x|r.candidates.iter().any(|c|c==&x.model)).map(|x|score_candidate(x,&r.weights)).collect(); scores.sort_by(|a,b|b.total.partial_cmp(&a.total).unwrap_or(std::cmp::Ordering::Equal).then_with(||a.model.cmp(&b.model))); let selected_model=scores.first().map(|x|x.model.clone()); let fallback=selected_model.is_none(); Ok(RouteDecision { request_id:r.request_id.clone(),mode:r.mode,selected_model,candidates_considered:r.candidates.clone(),scores,fallback,explanation:if fallback { "No eligible candidate; adapter fallback required".into() } else { "Selected highest deterministic weighted score".into() } }) }
fn sensitive(k:&str)->bool { let k=k.to_ascii_lowercase(); k.contains("api_key")||k.contains("apikey")||k.contains("token")||k.contains("password") }
fn redacted_metadata(m:&BTreeMap<String,String>)->BTreeMap<String,String> { m.iter().map(|(k,v)|(k.clone(),if sensitive(k){REDACTED.into()}else{v.clone()})).collect() }
fn serialize_redacted_metadata<S>(m:&BTreeMap<String,String>,s:S)->Result<S::Ok,S::Error> where S:Serializer { redacted_metadata(m).serialize(s) }

#[cfg(test)]
mod tests {
 use super::*;
 fn req(c:Vec<String>)->RouteRequest { RouteRequest { request_id:"r".into(),intent:"chat".into(),complexity:0.5,mode:RouteMode::Omni,candidates:c,weights:RouteWeights::default(),metadata:BTreeMap::new() } }
 #[test] fn defaults_to_omni(){let x:RouteRequest=serde_json::from_str(r#"{"request_id":"r1","intent":"chat","candidates":[]}"#).unwrap();assert_eq!(x.mode,RouteMode::Omni)}
 #[test] fn mode_roundtrips_as_lowercase(){for m in [RouteMode::Direct,RouteMode::Router,RouteMode::Ensemble,RouteMode::Bloom,RouteMode::Omni]{let j=serde_json::to_string(&m).unwrap();assert_eq!(serde_json::from_str::<RouteMode>(&j).unwrap(),m);assert_eq!(j,format!("\"{}\"",m.as_str()))}}
 #[test] fn unknown_mode_is_structured_error(){assert_eq!(RouteMode::parse("galaxy").unwrap_err().code(),"invalid_route_mode");assert!(serde_json::from_str::<RouteMode>(r#""galaxy""#).is_err())}
 #[test] fn request_validation_rejects_invalid_values(){let x=RouteRequest { request_id:" ".into(),intent:"chat".into(),complexity:0.5,mode:RouteMode::Omni,candidates:vec!["a".into(),"".into()],weights:RouteWeights::default(),metadata:BTreeMap::new() };let e=x.validate().unwrap_err();assert!(e.iter().any(|x|x.code()=="invalid_request_id"));assert!(e.iter().any(|x|x.code()=="invalid_candidate"));}
 #[test] fn default_and_custom_weights(){assert_eq!((RouteWeights::default().capability,RouteWeights::default().cost,RouteWeights::default().latency),(0.5,0.3,0.2));assert!(RouteWeights {capability:1.,cost:0.,latency:0.}.validate().is_ok())}
 #[test] fn scores_are_deterministic_and_sorted(){let x=req(vec!["slow".into(),"fast".into()]);let p=[CandidateProfile {model:"slow".into(),capability:0.9,cost:0.1,latency:0.1},CandidateProfile {model:"fast".into(),capability:0.8,cost:0.1,latency:0.9}];let a=decide(&x,&p).unwrap();assert_eq!(a,decide(&x,&p).unwrap());assert_eq!(a.selected_model.as_deref(),Some("slow"))}
 #[test] fn empty_candidates_produce_fallback(){let x=decide(&req(Vec::new()),&[]).unwrap();assert!(x.fallback);assert_eq!(x.selected_model,None)}
 #[test] fn metadata_is_redacted_in_debug_and_json(){let m=BTreeMap::from([(String::from("api_key"),String::from("secret-key")),(String::from("token"),String::from("secret-token")),(String::from("password"),String::from("secret-password")),(String::from("trace_id"),String::from("safe"))]);let x=RouteRequest {request_id:"r".into(),intent:"i".into(),complexity:0.,mode:RouteMode::Omni,candidates:vec!["m".into()],weights:RouteWeights::default(),metadata:m};let j=serde_json::to_string(&x).unwrap();let d=format!("{x:?}");for o in [&j,&d]{assert!(!o.contains("secret-key"));assert!(!o.contains("secret-token"));assert!(!o.contains("secret-password"));assert!(o.contains(REDACTED))}assert!(j.contains("trace_id"))}
}
