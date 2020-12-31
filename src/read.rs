use super::InvalidData;
use std::{io, ops::Deref};
use std::str::{from_utf8, FromStr};

use crate::{
    Command, Header, ReferenceIndex, Scope, ScopeItem, ScopeType, SimulationCommand, Value, Var,
};

fn whitespace_byte(b: u8) -> bool {
    match b {
        b' ' | b'\n' | b'\r' | b'\t' => true,
        _ => false,
    }
}

#[derive(Debug, PartialEq)]
pub struct ValueVector<'a> {
    v: &'a mut Vec<Value>,
}

impl<'a> Deref for ValueVector<'a> {
    type Target = [Value];

    fn deref(&self) -> &Self::Target {
        self.v
    }
}
impl<'a> Drop for ValueVector<'a> {
    fn drop(&mut self) {
        self.v.clear();
    }
}

// /// This must either be iterated through or dropped to
// /// continue using the parser.
// pub struct ValueVecIter<'a, R: io::Read> {
//     parser: &'a mut Parser<R>,
//     consumed: bool,
// }
// impl<'a, R: io::Read> Drop for ValueVecIter<'a, R> {
//     fn drop(&mut self) {
//         if !self.consumed {
//             let mut reached_data = false;
//             loop {
//                 let b = self.parser.read_byte().unwrap();

//                 if whitespace_byte(b) {
//                     if reached_data {
//                         break;
//                     } else {
//                         continue;
//                     }
//                 }
//                 reached_data = true;
//             }

//             let mut buf = [0; 32];
//             self.parser.read_token(&mut buf).unwrap();
//         }
//     }
// }

// impl<'a, R: io::Read> Iterator for ValueVecIter<'a, R> {
//     type Item = Value;

//     fn next(&mut self) -> Option<Value> {
//         if self.consumed {
//             return None;
//         }

//         // let mut val = Vec::new();
//         // loop {
//         //     let b = self.read_byte()?;
//         //     if whitespace_byte(b) {
//         //         if val.len() > 0 {
//         //             break;
//         //         } else {
//         //             continue;
//         //         }
//         //     }
//         //     val.push(Value::parse(b)?);
//         // }
//         // let id = self.read_token_parse()?;
//         // Ok(Command::ChangeVector(id, val))

//         // let b = self.parser.read_byte().unwrap();

//         todo!()
//     }
// }


/// VCD parser. Wraps an `io::Read` and acts as an iterator of `Command`s.
pub struct Parser<R: io::Read> {
    bytes_iter: io::Bytes<R>,
    simulation_command: Option<SimulationCommand>,
    value_change_vector_buffer: Vec<Value>,
}

impl<R: io::Read> Parser<R> {
    /// Creates a parser wrapping an `io::Read`.
    ///
    /// ```
    /// let buf = b"...";
    /// let mut vcd = vcd::Parser::new(&buf[..]);
    /// ```
    pub fn new(r: R) -> Parser<R> {
        Parser {
            bytes_iter: r.bytes(),
            simulation_command: None,
            value_change_vector_buffer: Vec::new(),
        }
    }

    fn read_byte(&mut self) -> Result<u8, io::Error> {
        match self.bytes_iter.next() {
            Some(b) => b,
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of VCD file",
            )),
        }
    }

    fn read_token<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a [u8], io::Error> {
        let mut len = 0;
        loop {
            let b = self.read_byte()?;
            if whitespace_byte(b) {
                if len > 0 {
                    break;
                } else {
                    continue;
                }
            }

            if let Some(p) = buf.get_mut(len) {
                *p = b;
            } else {
                return Err(InvalidData("token too long").into());
            }

            len += 1;
        }
        Ok(&buf[..len])
    }

    fn read_token_str<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a str, io::Error> {
        from_utf8(self.read_token(buf)?).map_err(|_| InvalidData("string is not UTF-8").into())
    }

    fn read_token_string(&mut self) -> Result<String, io::Error> {
        let mut r = Vec::new();
        loop {
            let b = self.read_byte()?;
            if whitespace_byte(b) {
                if r.len() > 0 {
                    break;
                } else {
                    continue;
                }
            }
            r.push(b);
        }
        String::from_utf8(r).map_err(|_| InvalidData("string is not UTF-8").into())
    }

    fn read_token_parse<T>(&mut self) -> Result<T, io::Error>
    where
        T: FromStr,
        <T as FromStr>::Err: 'static + ::std::error::Error + Send + Sync,
    {
        let mut buf = [0; 32];
        let tok = self.read_token_str(&mut buf)?;
        tok.parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    fn read_command_end(&mut self) -> Result<(), io::Error> {
        let mut buf = [0; 8];
        let tok = self.read_token(&mut buf)?;
        if tok == b"$end" {
            Ok(())
        } else {
            Err(InvalidData("expected $end").into())
        }
    }

    fn read_string_command(&mut self) -> Result<String, io::Error> {
        let mut r = Vec::new();
        loop {
            r.push(self.read_byte()?);
            if r.ends_with(b"$end") {
                break;
            }
        }
        let len = r.len() - 4;
        r.truncate(len);

        let s = from_utf8(&r).map_err(|_| io::Error::from(InvalidData("string is not UTF-8")))?;
        Ok(s.trim().to_string()) // TODO: don't reallocate
    }

    fn read_reference_index_end(&mut self) -> Result<Option<ReferenceIndex>, io::Error> {
        let mut buf = [0; 32];
        let tok = self.read_token_str(&mut buf)?;
        if tok.as_bytes() == b"$end" {
            return Ok(None);
        }

        let index: ReferenceIndex = tok.parse()?;
        self.read_command_end()?;
        Ok(Some(index))
    }

    fn parse_command(&mut self) -> Result<Command<'static>, io::Error> {
        use Command::*;
        use SimulationCommand::*;

        let mut cmdbuf = [0; 16];
        let cmd = self.read_token(&mut cmdbuf)?;

        match cmd {
            b"comment" => Ok(Comment(self.read_string_command()?)),
            b"date" => Ok(Date(self.read_string_command()?)),
            b"version" => Ok(Version(self.read_string_command()?)),
            b"timescale" => {
                let (mut buf, mut buf2) = ([0; 8], [0; 8]);
                let tok = self.read_token_str(&mut buf)?;
                // Support both "1ps" and "1 ps"
                let (num_str, unit_str) = match tok.find(|c: char| !c.is_numeric()) {
                    Some(idx) => (&tok[0..idx], &tok[idx..]),
                    None => (tok, self.read_token_str(&mut buf2)?),
                };
                self.read_command_end()?;
                let quantity = num_str
                    .parse()
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                let unit = unit_str.parse()?;
                Ok(Timescale(quantity, unit))
            }
            b"scope" => {
                let scope_type = self.read_token_parse()?;
                let identifier = self.read_token_string()?;
                self.read_command_end()?;
                Ok(ScopeDef(scope_type, identifier))
            }
            b"upscope" => {
                self.read_command_end()?;
                Ok(Upscope)
            }
            b"var" => {
                let var_type = self.read_token_parse()?;
                let size = self.read_token_parse()?;
                let code = self.read_token_parse()?;
                let reference = self.read_token_string()?;
                let index = self.read_reference_index_end()?;
                Ok(VarDef(var_type, size, code, reference, index))
            }
            b"enddefinitions" => {
                self.read_command_end()?;
                Ok(Enddefinitions)
            }

            // Simulation commands
            b"dumpall" => self.begin_simulation_command(Dumpall),
            b"dumpoff" => self.begin_simulation_command(Dumpoff),
            b"dumpon" => self.begin_simulation_command(Dumpon),
            b"dumpvars" => self.begin_simulation_command(Dumpvars),

            b"end" => {
                if let Some(c) = self.simulation_command.take() {
                    Ok(End(c))
                } else {
                    Err(InvalidData("unmatched $end").into())
                }
            }

            _ => Err(InvalidData("invalid keyword").into()),
        }
    }

    fn begin_simulation_command(&mut self, c: SimulationCommand) -> Result<Command<'static>, io::Error> {
        self.simulation_command = Some(c);
        Ok(Command::Begin(c))
    }

    fn parse_timestamp(&mut self) -> Result<Command<'static>, io::Error> {
        Ok(Command::Timestamp(self.read_token_parse()?))
    }

    fn parse_scalar(&mut self, initial: u8) -> Result<Command<'static>, io::Error> {
        let id = self.read_token_parse()?;
        let val = Value::parse(initial)?;
        Ok(Command::ChangeScalar(id, val))
    }

    fn parse_vector(&mut self) -> Result<Command, io::Error> {
        // let mut val = Vec::new();
        loop {
            let b = self.read_byte()?;
            if whitespace_byte(b) {
                if self.value_change_vector_buffer.len() > 0 {
                    break;
                } else {
                    continue;
                }
            }
            self.value_change_vector_buffer.push(Value::parse(b)?);
        }
        let id = self.read_token_parse()?;

        Ok(Command::ChangeVector(id, ValueVector {
            v: &mut self.value_change_vector_buffer,
        }))
    }

    fn parse_real(&mut self) -> Result<Command<'static>, io::Error> {
        let val = self.read_token_parse()?;
        let id = self.read_token_parse()?;
        Ok(Command::ChangeReal(id, val))
    }

    fn parse_string(&mut self) -> Result<Command<'static>, io::Error> {
        let val = self.read_token_string()?;
        let id = self.read_token_parse()?;
        Ok(Command::ChangeString(id, val))
    }

    fn parse_scope(
        &mut self,
        scope_type: ScopeType,
        reference: String,
    ) -> Result<Scope, io::Error> {
        use Command::*;
        let mut children = Vec::new();

        loop {
            match self.next_header_command() {
                Some(Ok(Upscope)) => break,
                Some(Ok(ScopeDef(tp, id))) => {
                    children.push(ScopeItem::Scope(self.parse_scope(tp, id)?));
                }
                Some(Ok(VarDef(tp, size, id, r, idx))) => {
                    children.push(ScopeItem::Var(Var {
                        var_type: tp,
                        size,
                        code: id,
                        reference: r,
                        index: idx,
                    }));
                }
                Some(Ok(_)) => return Err(InvalidData("unexpected command in $scope").into()),
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "unexpected EOF with open $scope",
                    ))
                }
            }
        }

        Ok(Scope {
            scope_type,
            identifier: reference,
            children,
        })
    }

    /// Parses the header of a VCD file into a `Header` struct.
    ///
    /// After returning, the stream has been read just past the `$enddefinitions` command and can
    /// be iterated to obtain the data.
    pub fn parse_header(&mut self) -> Result<Header, io::Error> {
        use Command::*;
        let mut header: Header = Default::default();
        loop {
            match self.next_header_command() {
                Some(Ok(Enddefinitions)) => break,
                Some(Ok(Comment(s))) => {
                    header.comment = Some(s);
                }
                Some(Ok(Date(s))) => {
                    header.date = Some(s);
                }
                Some(Ok(Version(s))) => {
                    header.version = Some(s);
                }
                Some(Ok(Timescale(val, unit))) => {
                    header.timescale = Some((val, unit));
                }
                Some(Ok(VarDef(var_type, size, code, reference, index))) => {
                    header.items.push(ScopeItem::Var(Var {
                        var_type,
                        size,
                        code,
                        reference,
                        index,
                    }));
                }
                Some(Ok(ScopeDef(tp, id))) => {
                    header
                        .items
                        .push(ScopeItem::Scope(self.parse_scope(tp, id)?));
                }
                Some(Ok(_)) => return Err(InvalidData("unexpected command in header").into()),
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "unexpected end of VCD file before $enddefinitions",
                    ))
                }
            }
        }
        Ok(header)
    }

    fn next_header_command(&mut self) -> Option<Result<Command<'static>, io::Error>> {
        while let Some(b) = self.bytes_iter.next() {
            let b = match b {
                Ok(b) => b,
                Err(e) => return Some(Err(e)),
            };
            match b {
                b' ' | b'\n' | b'\r' | b'\t' => (),
                b'$' => return Some(self.parse_command()),
                b'#' => return Some(self.parse_timestamp()),
                b'0' | b'1' | b'z' | b'Z' | b'x' | b'X' => return Some(self.parse_scalar(b)),
                // b'b' | b'B' => return Some(self.parse_vector()),
                b'r' | b'R' => return Some(self.parse_real()),
                b's' | b'S' => return Some(self.parse_string()),
                _ => {
                    return Some(Err(
                        InvalidData("unexpected character at start of command").into()
                    ))
                }
            }
        }
        None
    }

    /// Unfortunately, since Command has a lifetime, this cannot be an iterator.
    pub fn next_command(&mut self) -> Option<Result<Command, io::Error>> {
        while let Some(b) = self.bytes_iter.next() {
            let b = match b {
                Ok(b) => b,
                Err(e) => return Some(Err(e)),
            };
            match b {
                b' ' | b'\n' | b'\r' | b'\t' => (),
                b'$' => return Some(self.parse_command()),
                b'#' => return Some(self.parse_timestamp()),
                b'0' | b'1' | b'z' | b'Z' | b'x' | b'X' => return Some(self.parse_scalar(b)),
                b'b' | b'B' => return Some(self.parse_vector()),
                b'r' | b'R' => return Some(self.parse_real()),
                b's' | b'S' => return Some(self.parse_string()),
                _ => {
                    return Some(Err(
                        InvalidData("unexpected character at start of command").into()
                    ))
                }
            }
        }
        None
    }
}

// impl<'a, P: io::Read> Iterator for Parser<P> {
//     type Item = Result<Command<'a>, io::Error>;
//     fn next(&'a mut self) -> Option<Result<Command, io::Error>> {
//         while let Some(b) = self.bytes_iter.next() {
//             let b = match b {
//                 Ok(b) => b,
//                 Err(e) => return Some(Err(e)),
//             };
//             match b {
//                 b' ' | b'\n' | b'\r' | b'\t' => (),
//                 b'$' => return Some(self.parse_command()),
//                 b'#' => return Some(self.parse_timestamp()),
//                 b'0' | b'1' | b'z' | b'Z' | b'x' | b'X' => return Some(self.parse_scalar(b)),
//                 b'b' | b'B' => return Some(self.parse_vector()),
//                 b'r' | b'R' => return Some(self.parse_real()),
//                 b's' | b'S' => return Some(self.parse_string()),
//                 _ => {
//                     return Some(Err(
//                         InvalidData("unexpected character at start of command").into()
//                     ))
//                 }
//             }
//         }
//         None
//     }
// }

#[cfg(test)]
mod test {
    use crate::{
        Command::*, Parser, ReferenceIndex, ScopeItem, ScopeType, SimulationCommand::*,
        TimescaleUnit, Value::*, Var, VarType,
    };

    #[test]
    fn wikipedia_sample() {
        let sample = b"
        $date
        Date text.
        $end
        $version
        VCD generator text.
        $end
        $comment
        Any comment text.
        $end
        $timescale 100 ns $end
        $scope module logic $end
        $var wire 8 # data $end
        $var wire 1 $ data_valid $end
        $var wire 1 % en $end
        $var wire 1 & rx_en $end
        $var wire 1 ' tx_en $end
        $var wire 1 ( empty $end
        $var wire 1 ) underrun $end
        $upscope $end
        $enddefinitions $end
        $dumpvars
        bxxxxxxxx #
        x$
        0%
        x&
        x'
        1(
        0)
        $end
        #0
        b10000001 #
        0$
        1%
        #2211
        0'
        #2296
        b0 #
        1$
        #2302
        0$
        #2303
            ";

        let mut b = Parser::new(&sample[..]);

        let header = b.parse_header().unwrap();
        assert_eq!(header.comment, Some("Any comment text.".to_string()));
        assert_eq!(header.date, Some("Date text.".to_string()));
        assert_eq!(header.version, Some("VCD generator text.".to_string()));
        assert_eq!(header.timescale, Some((100, TimescaleUnit::NS)));

        let scope = match &header.items[0] {
            ScopeItem::Scope(sc) => sc,
            x => panic!("Expected Scope, found {:?}", x),
        };

        assert_eq!(&scope.identifier[..], "logic");
        assert_eq!(scope.scope_type, ScopeType::Module);

        if let ScopeItem::Var(ref v) = scope.children[0] {
            assert_eq!(v.var_type, VarType::Wire);
            assert_eq!(&v.reference[..], "data");
            assert_eq!(v.size, 8);
        } else {
            panic!("Expected Var, found {:?}", scope.children[0]);
        }

        let expected = &[
            Begin(Dumpvars),
            ChangeVector(2u32.into(), vec![X, X, X, X, X, X, X, X]),
            ChangeScalar(3u32.into(), X),
            ChangeScalar(4u32.into(), V0),
            ChangeScalar(5u32.into(), X),
            ChangeScalar(6u32.into(), X),
            ChangeScalar(7u32.into(), V1),
            ChangeScalar(8u32.into(), V0),
            End(Dumpvars),
            Timestamp(0),
            ChangeVector(2u32.into(), vec![V1, V0, V0, V0, V0, V0, V0, V1]),
            ChangeScalar(3u32.into(), V0),
            ChangeScalar(4u32.into(), V1),
            Timestamp(2211),
            ChangeScalar(6u32.into(), V0),
            Timestamp(2296),
            ChangeVector(2u32.into(), vec![V0]),
            ChangeScalar(3u32.into(), V1),
            Timestamp(2302),
            ChangeScalar(3u32.into(), V0),
            Timestamp(2303),
        ];

        for (i, e) in b.zip(expected.iter()) {
            assert_eq!(&i.unwrap(), e);
        }
    }

    #[test]
    fn name_with_spaces() {
        let sample = b"
$scope module top $end
$var wire 1 ! i_vld [0] $end
$var wire 10 ~ i_data [9:0] $end
$upscope $end
$enddefinitions $end
#0
";
        let mut parser = Parser::new(&sample[..]);
        let header = parser.parse_header().unwrap();

        let scope = match &header.items[0] {
            ScopeItem::Scope(sc) => sc,
            x => panic!("Expected Scope, found {:?}", x),
        };

        assert_eq!(&scope.identifier[..], "top");
        assert_eq!(scope.scope_type, ScopeType::Module);

        if let ScopeItem::Var(ref v) = scope.children[0] {
            assert_eq!(v.var_type, VarType::Wire);
            assert_eq!(&v.reference[..], "i_vld");
            assert_eq!(v.index, Some(ReferenceIndex::BitSelect(0)));
            assert_eq!(v.size, 1);
        } else {
            panic!("Expected Var, found {:?}", scope.children[0]);
        }

        if let ScopeItem::Var(ref v) = scope.children[1] {
            assert_eq!(v.var_type, VarType::Wire);
            assert_eq!(&v.reference[..], "i_data");
            assert_eq!(v.index, Some(ReferenceIndex::Range(9, 0)));
            assert_eq!(v.size, 10);
        } else {
            panic!("Expected Var, found {:?}", scope.children[0]);
        }
    }

    #[test]
    fn more_type_examples() {
        let sample = b"
$scope module logic $end
$var integer 32 t smt_step $end
$var event 1 ! smt_clock $end
$upscope $end
$enddefinitions $end
#0
1!
b00000000000000000000000000000000 t
";
        let mut b = Parser::new(&sample[..]);

        let header = b.parse_header().unwrap();
        assert_eq!(header.comment, None);
        assert_eq!(header.date, None);
        assert_eq!(header.version, None);
        assert_eq!(header.timescale, None);

        let scope = match &header.items[0] {
            ScopeItem::Scope(sc) => sc,
            x => panic!("Expected Scope, found {:?}", x),
        };

        assert_eq!(&scope.identifier[..], "logic");
        assert_eq!(scope.scope_type, ScopeType::Module);

        if let ScopeItem::Var(ref v) = scope.children[0] {
            assert_eq!(v.var_type, VarType::Integer);
            assert_eq!(&v.reference[..], "smt_step");
            assert_eq!(v.size, 32);
        } else {
            panic!("Expected Var, found {:?}", scope.children[0]);
        }

        let expected = &[
            Timestamp(0),
            ChangeScalar(0u32.into(), V1),
            ChangeVector(83u32.into(), vec![V0; 32]),
        ];

        for (i, e) in b.zip(expected.iter()) {
            assert_eq!(&i.unwrap(), e);
        }
    }

    #[test]
    fn symbiyosys_example() {
        let sample = b"
$var integer 32 t smt_step $end
$var event 1 ! smt_clock $end
$scope module queue $end
$var wire 8 n11 buffer<0> $end
$var wire 1 n0 i_clk $end
$var wire 8 n1 i_data $end
$var wire 1 n2 i_read_enable $end
$var wire 1 n3 i_reset_n $end
$var wire 1 n4 i_write_enable $end
$var wire 1 n5 matches $end
$var wire 8 n6 o_data $end
$var wire 1 n7 o_empty $end
$var wire 1 n8 o_full $end
$var wire 5 n9 r_ptr $end
$var wire 5 n10 w_ptr $end
$upscope $end
$enddefinitions $end
#0
1!
b00000000000000000000000000000000 t
b1 n0
b00000000 n1
b0 n2
b0 n3
b0 n4
b1 n5
b00000000 n6
b1 n7
b0 n8
b00000 n9
b00000 n10
b00000000 n11
#5
b0 n0
#10
1!
b00000000000000000000000000000001 t
b1 n0
b00000000 n1
b0 n2
b0 n3
b0 n4
b1 n5
b00000000 n6
b1 n7
b0 n8
b00000 n9
b00000 n10
b00000000 n11
#15
b0 n0
#20
1!
b00000000000000000000000000000010 t
b1 n0
";

        let mut b = Parser::new(&sample[..]);

        let header = b.parse_header().unwrap();
        assert_eq!(header.comment, None);
        assert_eq!(header.date, None);
        assert_eq!(header.version, None);
        assert_eq!(header.timescale, None);

        assert_eq!(
            header.items[0],
            ScopeItem::Var(Var {
                var_type: VarType::Integer,
                size: 32,
                code: 83u32.into(),
                reference: "smt_step".into(),
                index: None,
            })
        );
        assert_eq!(
            header.items[1],
            ScopeItem::Var(Var {
                var_type: VarType::Event,
                size: 1,
                code: 0u32.into(),
                reference: "smt_clock".into(),
                index: None,
            })
        );

        let scope = match &header.items[2] {
            ScopeItem::Scope(sc) => sc,
            x => panic!("Expected Scope, found {:?}", x),
        };

        assert_eq!(&scope.identifier[..], "queue");
        assert_eq!(scope.scope_type, ScopeType::Module);

        if let ScopeItem::Var(ref v) = scope.children[0] {
            assert_eq!(v.var_type, VarType::Wire);
            assert_eq!(&v.reference[..], "buffer<0>");
            assert_eq!(v.size, 8);
        } else {
            panic!("Expected Var, found {:?}", scope.children[0]);
        }

        let expected = &[
            Timestamp(0),
            ChangeScalar(0u32.into(), V1),
            ChangeVector(
                83u32.into(),
                vec![
                    V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0,
                    V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0,
                ],
            ),
            ChangeVector(1581u32.into(), vec![V1]),
            ChangeVector(1675u32.into(), vec![V0, V0, V0, V0, V0, V0, V0, V0]),
            ChangeVector(1769u32.into(), vec![V0]),
            ChangeVector(1863u32.into(), vec![V0]),
            ChangeVector(1957u32.into(), vec![V0]),
            ChangeVector(2051u32.into(), vec![V1]),
            ChangeVector(2145u32.into(), vec![V0, V0, V0, V0, V0, V0, V0, V0]),
            ChangeVector(2239u32.into(), vec![V1]),
            ChangeVector(2333u32.into(), vec![V0]),
            ChangeVector(2427u32.into(), vec![V0, V0, V0, V0, V0]),
            ChangeVector(143051u32.into(), vec![V0, V0, V0, V0, V0]),
            ChangeVector(151887u32.into(), vec![V0, V0, V0, V0, V0, V0, V0, V0]),
            Timestamp(5),
            ChangeVector(1581u32.into(), vec![V0]),
            Timestamp(10),
            ChangeScalar(0u32.into(), V1),
            ChangeVector(
                83u32.into(),
                vec![
                    V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0,
                    V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V1,
                ],
            ),
            ChangeVector(1581u32.into(), vec![V1]),
            ChangeVector(1675u32.into(), vec![V0, V0, V0, V0, V0, V0, V0, V0]),
            ChangeVector(1769u32.into(), vec![V0]),
            ChangeVector(1863u32.into(), vec![V0]),
            ChangeVector(1957u32.into(), vec![V0]),
            ChangeVector(2051u32.into(), vec![V1]),
            ChangeVector(2145u32.into(), vec![V0, V0, V0, V0, V0, V0, V0, V0]),
            ChangeVector(2239u32.into(), vec![V1]),
            ChangeVector(2333u32.into(), vec![V0]),
            ChangeVector(2427u32.into(), vec![V0, V0, V0, V0, V0]),
            ChangeVector(143051u32.into(), vec![V0, V0, V0, V0, V0]),
            ChangeVector(151887u32.into(), vec![V0, V0, V0, V0, V0, V0, V0, V0]),
            Timestamp(15),
            ChangeVector(1581u32.into(), vec![V0]),
            Timestamp(20),
            ChangeScalar(0u32.into(), V1),
            ChangeVector(
                83u32.into(),
                vec![
                    V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V0,
                    V0, V0, V0, V0, V0, V0, V0, V0, V0, V0, V1, V0,
                ],
            ),
            ChangeVector(1581u32.into(), vec![V1]),
        ];

        for (i, e) in b.zip(expected.iter()) {
            assert_eq!(&i.unwrap(), e);
        }
    }
}
