//! Prints every layer id in the recorder dump; used to check which rows the table can back.
fn main() {
    match haste_entity::model_dump::library() {
        Some(lib) => {
            for id in lib.layer_ids() {
                println!("{id}");
            }
        }
        None => eprintln!("no dump"),
    }
}
