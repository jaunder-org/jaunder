fn main() {
    traced_run::traced_cli(std::env::args().skip(1).collect());
}
