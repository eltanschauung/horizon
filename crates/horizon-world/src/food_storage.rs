//! FoodData fields in a vanilla player NBT compound.
use crate::{
    food::FoodData,
    nbt::{Compound, JavaString, Tag},
};
fn int(tag: Option<&Tag>, default: i32) -> i32 {
    match tag {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        Some(Tag::Float(v)) => f32::from_bits(*v).floor() as i32,
        Some(Tag::Double(v)) => f64::from_bits(*v).floor() as i32,
        _ => default,
    }
}
fn float(tag: Option<&Tag>, default: f32) -> f32 {
    match tag {
        Some(Tag::Byte(v)) => f32::from(*v),
        Some(Tag::Short(v)) => f32::from(*v),
        Some(Tag::Int(v)) => *v as f32,
        Some(Tag::Long(v)) => *v as f32,
        Some(Tag::Float(v)) => f32::from_bits(*v),
        Some(Tag::Double(v)) => f64::from_bits(*v) as f32,
        _ => default,
    }
}
impl FoodData {
    /// Missing or nonnumeric values reset each field to its native default.
    /// Numeric values are converted without clamping the resulting food state.
    pub fn read_nbt(&mut self, tag: &Compound) {
        let get = |s: &str| tag.get(&JavaString::from(s));
        self.max_food = int(get("horizon:foodCapacity"), 20).clamp(1,100);
        self.food = int(get("foodLevel"), 20);
        self.timer = int(get("foodTickTimer"), 0);
        self.saturation = float(get("foodSaturationLevel"), 5.0);
        self.exhaustion = float(get("foodExhaustionLevel"), 0.0);
    }
    /// Writes these four fields while preserving other player data. Java float
    /// tag factories normalize both signed zeros to the shared positive zero.
    pub fn write_nbt(&self, tag: &mut Compound) {
        let float = |v: f32| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() });
        if self.max_food != 20 {
            tag.insert("horizon:foodCapacity".into(), Tag::Int(self.max_food));
        } else { tag.remove(&JavaString::from("horizon:foodCapacity")); }
        tag.insert("foodLevel".into(), Tag::Int(self.food));
        tag.insert("foodTickTimer".into(), Tag::Int(self.timer));
        tag.insert("foodSaturationLevel".into(), float(self.saturation));
        tag.insert("foodExhaustionLevel".into(), float(self.exhaustion));
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn nonstandard_capacity_roundtrips_without_refilling_hunger() {
        let mut food=FoodData::default(); food.set_capacity(30); food.food=25;
        let mut tag=Compound::new(); food.write_nbt(&mut tag);
        let mut loaded=FoodData::default(); loaded.read_nbt(&tag);
        loaded.set_capacity(30);
        assert_eq!((loaded.food,loaded.max_food),(25,30));
        let mut vanilla=Compound::new(); FoodData::default().write_nbt(&mut vanilla);
        assert!(!vanilla.contains_key(&JavaString::from("horizon:foodCapacity")));
    }
}
