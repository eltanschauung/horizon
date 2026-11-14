//! Filesystem/registry reproduction only: no game, window, audio device or player data.
use horizon_client_render::{packs,sound_events::SoundRegistry};
use std::path::PathBuf;
fn main(){
 let args:Vec<_>=std::env::args_os().collect();assert_eq!(args.len(),3,"BASE_RESOURCES EMPTY_SCRATCH");
 let base=PathBuf::from(&args[1]);let scratch=PathBuf::from(&args[2]);assert!(!scratch.exists(),"Choose fresh scratch directory");std::fs::create_dir_all(&scratch).unwrap();
 let all=packs::discover(&base,&scratch.join("packs"));
 for ids in [vec!["programmer_art".into()],vec!["programmer_art".into(),"pacp".into()]]{
  let selected=packs::enabled(&all,&ids);let out=scratch.join(format!("overlay-{}",ids.len()));packs::build_overlay(&base,&selected,&out).unwrap();
  let registry=SoundRegistry::load(&out);
  for event in ["minecraft:block.stone.break","minecraft:block.grass.break","minecraft:entity.player.hurt"]{
   let entry=registry.get(event).expect("Event missing after pack reload");assert!(!entry.sounds.is_empty(),"{event}: no variants");
   for sound in entry.sounds.iter().filter(|s|!s.event){let (ns,path)=sound.path();let path=out.join("assets").join(ns).join(path);let bytes=std::fs::read(&path).unwrap_or_else(|e|panic!("{}: {e}",path.display()));assert!(bytes.starts_with(b"OggS"));}
   println!("{}: {event}: {} variants readable",ids.join("+"),entry.sounds.len());
  }
 }
}
