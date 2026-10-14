//! Prints one layer's parts (name, pivot, cubes) from the dump; geometry debugging.
fn main() {
    let id = std::env::args().nth(1).expect("layer id argument");
    let Some(lib) = haste_entity::model_dump::library() else {
        eprintln!("no dump");
        return;
    };
    let Some(m) = lib.get(&id, 1.0) else {
        eprintln!("no layer {id}");
        return;
    };
    fn walk(p: &haste_core::model::ModelPart, depth: usize) {
        println!(
            "{:indent$}{} pivot {:?} rot {:?} cubes {}",
            "",
            p.name,
            p.pivot,
            p.rotation,
            p.cubes.len(),
            indent = depth * 2
        );
        for c in &p.cubes {
            println!(
                "{:indent$}  cube origin {:?} size {:?} uv {:?}",
                "",
                c.origin,
                c.size,
                c.uv,
                indent = depth * 2
            );
        }
        for c in &p.children {
            walk(c, depth + 1);
        }
    }
    println!("tex {}x{}", m.tex_width, m.tex_height);
    walk(&m.root, 0);
}
