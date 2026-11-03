//! Durable per-account/per-character name reservations. Slots never get recycled.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::{self, Write}, path::Path};
#[derive(Default,Serialize,Deserialize)]
pub struct Reservations {
    #[serde(default)] pub joins: BTreeMap<String,u64>,
    #[serde(default)] pub characters: BTreeMap<String,BTreeMap<String,u32>>,
}
impl Reservations {
    pub fn load(path:&Path)->io::Result<Self>{match std::fs::read(path){Ok(b)=>serde_json::from_slice(&b).map_err(io::Error::other),Err(e)if e.kind()==io::ErrorKind::NotFound=>Ok(Self::default()),Err(e)=>Err(e)}}
    pub fn join(&mut self,uuid:u128){let next=self.joins.values().max().map_or(0,|n|n+1);self.joins.entry(format!("{uuid:032x}")).or_insert(next);}
    pub fn reserve(&mut self,uuid:u128,character:&str)->u32{self.join(uuid);let claims=self.characters.entry(character.into()).or_default();let next=claims.values().max().map_or(0,|n|n+1);*claims.entry(format!("{uuid:032x}")).or_insert(next)}
    pub fn save(&self,path:&Path)->io::Result<()>{let temp=path.with_extension(format!("tmp-{}",std::process::id()));let result=(||{let mut f=std::fs::OpenOptions::new().create_new(true).write(true).open(&temp)?;f.write_all(&serde_json::to_vec_pretty(self)?)?;f.sync_all()?;drop(f);std::fs::rename(&temp,path)})();if result.is_err(){let _=std::fs::remove_file(temp);}result}
}
pub fn suffix(mut slot:u32)->String{if slot==0{return String::new();}slot+=1;let mut letters=Vec::new();while slot>0{slot-=1;letters.push((b'A'+(slot%26)as u8)as char);slot/=26;}format!(" ({})",letters.iter().rev().collect::<String>())}
