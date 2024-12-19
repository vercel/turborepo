use quickcheck::Arbitrary as _;

mod helpers;

#[derive(Clone, Debug)]
struct TerminalInput(Vec<u8>);

fn gen_range<T>(gen: &mut quickcheck::Gen, range: std::ops::Range<T>) -> T
where
    T: Copy,
    T: quickcheck::Arbitrary,
    T: std::ops::Add<Output = T>
        + std::ops::Rem<Output = T>
        + std::ops::Sub<Output = T>,
{
    T::arbitrary(gen) % (range.end - range.start) + range.start
}

impl quickcheck::Arbitrary for TerminalInput {
    fn arbitrary(g: &mut quickcheck::Gen) -> Self {
        let size = {
            let s = g.size();
            gen_range(g, 0..s)
        };
        TerminalInput(
            (0..size)
                .flat_map(|_| choose_terminal_input_fragment(g))
                .collect(),
        )
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(self.0.shrink().map(TerminalInput))
    }
}

fn choose_terminal_input_fragment(g: &mut quickcheck::Gen) -> Vec<u8> {
    #[derive(Clone)]
    enum Fragment {
        Text,
        Control,
        Escape,
        Csi,
        #[allow(dead_code)]
        Osc,
        #[allow(dead_code)]
        Dcs,
    }

    impl quickcheck::Arbitrary for Fragment {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            match u8::arbitrary(g) {
                0u8..=231 => Fragment::Text,
                232..=239 => Fragment::Control,
                240..=247 => Fragment::Escape,
                248..=255 => Fragment::Csi,
            }
        }
    }

    match Fragment::arbitrary(g) {
        Fragment::Text => {
            let mut u: u32 = gen_range(g, 32..(2u32.pow(20) - 2048));
            // surrogates aren't valid codepoints on their own
            if u >= 0xD800 {
                u += 2048;
            }
            let c: Result<char, _> = std::convert::TryFrom::try_from(u);
            let c = match c {
                Ok(c) => c,
                Err(e) => panic!("failed to create char from {u}: {e}"),
            };
            let mut b = [0; 4];
            let s = c.encode_utf8(&mut b);
            (*s).to_string().into_bytes()
        }
        Fragment::Control => vec![gen_range(g, 7..14)],
        Fragment::Escape => {
            let mut v = vec![0x1b];
            let c = gen_range(g, b'0'..b'~');
            v.push(c);
            v
        }
        Fragment::Csi => {
            let mut v = vec![0x1b, b'['];
            // TODO: params
            let c = gen_range(g, b'@'..b'~');
            v.push(c);
            v
        }
        Fragment::Osc => {
            // TODO
            unimplemented!()
        }
        Fragment::Dcs => {
            // TODO
            unimplemented!()
        }
    }
    // TODO: sometimes add garbage in random places
}

fn contents_formatted_reproduces_state_random(input: Vec<u8>) -> bool {
    helpers::contents_formatted_reproduces_state(&input)
}

fn contents_formatted_reproduces_state_structured(
    input: TerminalInput,
) -> bool {
    helpers::contents_formatted_reproduces_state(&input.0)
}

#[test]
#[ignore]
fn qc_structured_long() {
    let mut qc = quickcheck::QuickCheck::new()
        .tests(1_000_000)
        .max_tests(1_000_000);
    qc.quickcheck(
        contents_formatted_reproduces_state_structured
            as fn(TerminalInput) -> bool,
    );
}

#[test]
fn qc_structured_short() {
    let mut qc = quickcheck::QuickCheck::new().tests(1_000).max_tests(1_000);
    qc.quickcheck(
        contents_formatted_reproduces_state_structured
            as fn(TerminalInput) -> bool,
    );
}

#[test]
#[ignore]
fn qc_random_long() {
    let mut qc = quickcheck::QuickCheck::new()
        .tests(10_000_000)
        .max_tests(10_000_000);
    qc.quickcheck(
        contents_formatted_reproduces_state_random as fn(Vec<u8>) -> bool,
    );
}

#[test]
fn qc_random_short() {
    let mut qc = quickcheck::QuickCheck::new()
        .tests(10_000)
        .max_tests(10_000);
    qc.quickcheck(
        contents_formatted_reproduces_state_random as fn(Vec<u8>) -> bool,
    );
}
