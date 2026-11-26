//! A small Vim-script expression evaluator for the expression register
//! (`Ctrl-R =` in Insert mode and on the command line) and `:echo`:
//! integers and floats, `'single'` and `"double"` quoted strings (`\n`, `\t`,
//! `\"` escapes in double quotes), `+ - * / %`, `.` and `..` concatenation,
//! unary minus and parentheses. Integer division truncates as in Vim.

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Int(i64),
    Float(f64),
    Str(String),
}

impl Value {
    fn render(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => {
                let s = format!("{f}");
                if s.contains(['.', 'e', 'i', 'N']) {
                    s
                } else {
                    format!("{s}.0")
                }
            }
            Value::Str(s) => s.clone(),
        }
    }
    fn number(&self) -> Result<Value, String> {
        match self {
            Value::Str(s) => {
                let t = s.trim();
                let digits: String = t
                    .chars()
                    .enumerate()
                    .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && *c == '-'))
                    .map(|(_, c)| c)
                    .collect();
                Ok(Value::Int(digits.parse().unwrap_or(0)))
            }
            v => Ok(v.clone()),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

/// Evaluates `src`, returning its value as text, or an error message.
pub fn eval(src: &str) -> Result<String, String> {
    let mut p = Parser {
        s: src.as_bytes(),
        i: 0,
    };
    let v = p.concat()?;
    p.ws();
    if p.i < p.s.len() {
        return Err(format!("E15: Invalid expression: \"{src}\""));
    }
    Ok(v.render())
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    /// `a . b`, `a .. b`: lowest precedence, as in Vim.
    fn concat(&mut self) -> Result<Value, String> {
        let mut v = self.sum()?;
        while self.peek() == Some(b'.') && !self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
            if self.s.get(self.i) == Some(&b'.') {
                self.i += 1;
            }
            let r = self.sum()?;
            v = Value::Str(v.render() + &r.render());
        }
        Ok(v)
    }
    fn sum(&mut self) -> Result<Value, String> {
        let mut v = self.product()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.product()?;
            v = arith(op, v, r)?;
        }
        Ok(v)
    }
    fn product(&mut self) -> Result<Value, String> {
        let mut v = self.unary()?;
        while let Some(op @ (b'*' | b'/' | b'%')) = self.peek() {
            self.i += 1;
            let r = self.unary()?;
            v = arith(op, v, r)?;
        }
        Ok(v)
    }
    fn unary(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                arith(b'-', Value::Int(0), self.unary()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.unary()?.number()
            }
            _ => self.atom(),
        }
    }
    fn atom(&mut self) -> Result<Value, String> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let v = self.concat()?;
                if self.peek() != Some(b')') {
                    return Err("E110: Missing ')'".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(b'\'') => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    match self.s.get(self.i) {
                        None => return Err("E115: Missing quote".into()),
                        // `''` is a literal quote inside single quotes.
                        Some(b'\'') if self.s.get(self.i + 1) == Some(&b'\'') => {
                            out.push(b'\'');
                            self.i += 2;
                        }
                        Some(b'\'') => {
                            self.i += 1;
                            break;
                        }
                        Some(&c) => {
                            out.push(c);
                            self.i += 1;
                        }
                    }
                }
                Ok(Value::Str(String::from_utf8_lossy(&out).into_owned()))
            }
            Some(b'"') => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    match self.s.get(self.i) {
                        None => return Err("E114: Missing quote".into()),
                        Some(b'"') => {
                            self.i += 1;
                            break;
                        }
                        Some(b'\\') => {
                            let c = self.s.get(self.i + 1).copied().unwrap_or(b'\\');
                            out.push(match c {
                                b'n' => b'\n',
                                b't' => b'\t',
                                b'r' => b'\r',
                                c => c,
                            });
                            self.i += 2;
                        }
                        Some(&c) => {
                            out.push(c);
                            self.i += 1;
                        }
                    }
                }
                Ok(Value::Str(String::from_utf8_lossy(&out).into_owned()))
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'.')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    return i64::from_str_radix(hex, 16)
                        .map(Value::Int)
                        .map_err(|_| format!("E15: Invalid number: {t}"));
                }
                if t.contains('.') {
                    t.parse::<f64>()
                        .map(Value::Float)
                        .map_err(|_| format!("E15: Invalid number: {t}"))
                } else {
                    t.parse::<i64>()
                        .map(Value::Int)
                        .map_err(|_| format!("E15: Invalid number: {t}"))
                }
            }
            _ => Err("E15: Invalid expression".into()),
        }
    }
}

fn arith(op: u8, a: Value, b: Value) -> Result<Value, String> {
    let (a, b) = (a.number()?, b.number()?);
    Ok(match (a, b) {
        (Value::Int(x), Value::Int(y)) => Value::Int(match op {
            b'+' => x.wrapping_add(y),
            b'-' => x.wrapping_sub(y),
            b'*' => x.wrapping_mul(y),
            b'/' if y == 0 => return Err("E1154: Divide by zero".into()),
            b'/' => x / y,
            b'%' if y == 0 => return Err("E1154: Divide by zero".into()),
            _ => x % y,
        }),
        (x, y) => {
            let f = |v: Value| match v {
                Value::Int(n) => n as f64,
                Value::Float(f) => f,
                Value::Str(_) => 0.0,
            };
            let (x, y) = (f(x), f(y));
            Value::Float(match op {
                b'+' => x + y,
                b'-' => x - y,
                b'*' => x * y,
                b'/' => x / y,
                _ => x % y,
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::eval;

    #[test]
    fn arithmetic_strings_and_errors() {
        assert_eq!(eval("2*21").unwrap(), "42");
        assert_eq!(eval("(1 + 2) * -3").unwrap(), "-9");
        assert_eq!(eval("7 / 2").unwrap(), "3");
        assert_eq!(eval("7 % 3").unwrap(), "1");
        assert_eq!(eval("1.5 * 2").unwrap(), "3.0");
        assert_eq!(eval("0x10 + 1").unwrap(), "17");
        assert_eq!(eval("'a' . \"b\" .. 1+1").unwrap(), "ab2");
        assert_eq!(eval("\"x\\ty\"").unwrap(), "x\ty");
        assert_eq!(eval("'it''s'").unwrap(), "it's");
        assert!(eval("1 / 0").is_err());
        assert!(eval("(1").is_err());
        assert!(eval("1 2").is_err());
    }
}
