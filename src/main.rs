use herdr_simple_prompts::{Mode, run_toggle, run_ui};

fn main() {
    let result = match Mode::parse(std::env::args().nth(1).as_deref()) {
        Ok(Mode::Toggle) => run_toggle(),
        Ok(Mode::Ui) => run_ui(),
        Err(error) => Err(error),
    };

    if let Err(error) = result {
        eprintln!("herdr-simple-prompts: {error}");
        std::process::exit(1);
    }
}
