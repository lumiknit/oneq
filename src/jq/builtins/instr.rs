//! Scalar instruction identity, arity and direct target in one list.
//! Both the enum and the VM match are expanded from this list: adding a scalar
//! cannot silently omit its execution arm or use a different argument count.
macro_rules! scalar_instructions {
    ($emit:ident) => {
        $emit! {
            Acos(0) => math::acos,
            Acosh(0) => math::acosh,
            Asin(0) => math::asin,
            Asinh(0) => math::asinh,
            Atan(0) => math::atan,
            Atanh(0) => math::atanh,
            Cos(0) => math::cos,
            Cosh(0) => math::cosh,
            Sin(0) => math::sin,
            Sinh(0) => math::sinh,
            Tan(0) => math::tan,
            Tanh(0) => math::tanh,
            Exp(0) => math::exp,
            Exp2(0) => math::exp2,
            Expm1(0) => math::expm1,
            Log(0) => math::log,
            Log2(0) => math::log2,
            Log10(0) => math::log10,
            Log1p(0) => math::log1p,
            Sqrt(0) => math::sqrt,
            Cbrt(0) => math::cbrt,
            Floor(0) => math::floor,
            Ceil(0) => math::ceil,
            Trunc(0) => math::trunc,
            Round(0) => math::round,
            Fabs(0) => math::fabs,
            Erf(0) => math::erf,
            Erfc(0) => math::erfc,
            Lgamma(0) => math::lgamma,
            Tgamma(0) => math::tgamma,
            J0(0) => math::j0,
            J1(0) => math::j1,
            Y0(0) => math::y0,
            Y1(0) => math::y1,
            Exp10(0) => math::exp10,
            Significand(0) => math::significand,
            Logb(0) => math::logb,
            Nearbyint(0) => math::nearbyint,
            Rint(0) => math::rint,
            Frexp(0) => math::frexp,
            Modf(0) => math::modf,
            Isnan(0) => math::isnan,
            Isinfinite(0) => math::isinfinite,
            Isnormal(0) => math::isnormal,
            Atan2(2) => math::atan2,
            Pow(2) => math::pow,
            Hypot(2) => math::hypot,
            Fmod(2) => math::fmod,
            Remainder(2) => math::remainder,
            Copysign(2) => math::copysign,
            Nextafter(2) => math::nextafter,
            Fdim(2) => math::fdim,
            Fmax(2) => math::fmax,
            Fmin(2) => math::fmin,
            Ldexp(2) => math::ldexp,
            Jn(2) => math::jn,
            Yn(2) => math::yn,
            Fma(3) => math::fma,
            Nan(0) => math::nan,
            LgammaR(0) => math::lgamma_r,
            Ilogb(0) => math::ilogb,
            Scalb(2) => math::scalb,
            Infinite(0) => math::infinite,
            Builtins(0) => scalar::builtins,
            MatchImpl(3) => regex::match_impl,
            CaptureImpl(2) => regex::capture_impl,
            Sort(0) => collections::sort,
            Unique(0) => collections::unique,
            Min(0) => collections::min,
            Max(0) => collections::max,
            Contains(1) => collections::contains,
            SortByKeys(0) => collections::sort_by_keys,
            GroupSorted(0) => collections::group_sorted,
            Utf8bytelength(0) => strings::utf8bytelength,
            Explode(0) => strings::explode,
            Implode(0) => strings::implode,
            AsciiDowncase(0) => strings::ascii_downcase,
            AsciiUpcase(0) => strings::ascii_upcase,
            Startswith(1) => strings::startswith,
            Endswith(1) => strings::endswith,
            Trim(0) => strings::trim,
            Ltrim(0) => strings::ltrim,
            Rtrim(0) => strings::rtrim,
            Split(1) => strings::split,
            Bsearch(1) => strings::bsearch,
            Text(0) => strings::text,
            AsJson(0) => strings::as_json,
            Html(0) => strings::html,
            Htmld(0) => strings::htmld,
            Uri(0) => strings::uri,
            Urid(0) => strings::urid,
            Base64(0) => strings::base64,
            Base64d(0) => strings::base64d,
            Base32(0) => strings::base32,
            Base32d(0) => strings::base32d,
            Hex(0) => strings::hex,
            Hexd(0) => strings::hexd,
            Ascii(0) => strings::ascii,
            Sh(0) => strings::sh,
            Csv(0) => strings::csv,
            Tsv(0) => strings::tsv,
            Sha1(0) => strings::sha1,
            Sha256(0) => strings::sha256,
            Sha512(0) => strings::sha512,
            GetPath(1) => scalar::getpath,
            DelPaths(1) => scalar::delpaths,
            SetPath(2) => scalar::setpath,
            Strindices(1) => strings::strindices,
            SortByImpl(1) => collections::sort_by_impl,
            GroupByImpl(1) => collections::group_by_impl,
            MaxByImpl(1) => collections::max_by_impl,
            MinByImpl(1) => collections::min_by_impl,
            UniqueByImpl(1) => collections::unique_by_impl,
            Sub(2) => scalar::subtract,
            Neg(1) => scalar::negate,
            Mul(2) => scalar::multiply,
            Div(2) => scalar::divide,
            Rem(2) => scalar::modulo,
            Eq(2) => scalar::equal,
            Ne(2) => scalar::unequal,
            Lt(2) => scalar::less,
            Le(2) => scalar::less_equal,
            Gt(2) => scalar::greater,
            Ge(2) => scalar::greater_equal,
            Not(0) => scalar::not,
            Tostring(0) => scalar::tostring,
            Tojson(0) => scalar::tojson,
            Fromjson(0) => scalar::fromjson,
            Tonumber(0) => scalar::tonumber,
            Keys(0) => scalar::keys,
            KeysUnsorted(0) => scalar::keys_unsorted,
            Has(1) => scalar::has,
            Abs(0) => scalar::abs,
            Gmtime(0) => datetime::gmtime,
            Localtime(0) => datetime::localtime,
            Mktime(0) => datetime::mktime,
            Strftime(1) => datetime::strftime,
            Strflocaltime(1) => datetime::strflocaltime,
            Strptime(1) => datetime::strptime,
            Random(0) => random::random,
            Randint2(2) => random::randint2,
            Choice(0) => random::choice,
            Type(0) => scalar::type_name,
            Length(0) => scalar::length,
            Now(0) => datetime::now,
            Debug(0) => scalar::debug,
            Stderr(0) => scalar::stderr,
        }
    };
}
pub(crate) use scalar_instructions;

macro_rules! define_instructions {
    ($($variant:ident($arity:literal) => $module:ident::$function:ident,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum BuiltinInstr {
            $($variant,)*
            Add, Range, Path, Empty, Error, Halt, HaltError,
            Input, Env, InputFilename, InputLineNumber, ModuleMeta,
            HaveDecnum, HaveLiteralNumbers,
        }
        impl BuiltinInstr {
            pub const MAX_ARITY: usize = {
                let mut max = 2;
                let arities = [$($arity,)*];
                let mut index = 0;
                while index < arities.len() {
                    if arities[index] > max { max = arities[index]; }
                    index += 1;
                }
                max
            };
            pub const fn arity(self) -> usize {
                match self {
                    $(Self::$variant => $arity,)*
                    Self::Add | Self::Range => 2,
                    Self::Path | Self::HaltError => 1,
                    Self::Empty | Self::Error | Self::Halt | Self::Input |
                    Self::Env | Self::InputFilename | Self::InputLineNumber |
                    Self::ModuleMeta | Self::HaveDecnum | Self::HaveLiteralNumbers => 0,
                }
            }
            pub const fn is_scalar(self) -> bool {
                matches!(self, $(Self::$variant |)* Self::Add | Self::HaveDecnum | Self::HaveLiteralNumbers)
            }
        }
    };
}
scalar_instructions!(define_instructions);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuiltinOp0(pub BuiltinInstr);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuiltinOp1(pub BuiltinInstr);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuiltinOp2(pub BuiltinInstr);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuiltinOp3(pub BuiltinInstr);

impl BuiltinInstr {
    #[must_use]
    pub const fn op0(self) -> BuiltinOp0 {
        BuiltinOp0(self)
    }
    #[must_use]
    pub const fn op1(self) -> BuiltinOp1 {
        BuiltinOp1(self)
    }
    #[must_use]
    pub const fn op2(self) -> BuiltinOp2 {
        BuiltinOp2(self)
    }
    #[must_use]
    pub const fn op3(self) -> BuiltinOp3 {
        BuiltinOp3(self)
    }

    #[must_use]
    pub const fn is_infix_operator(self) -> bool {
        matches!(
            self,
            Self::Add
                | Self::Sub
                | Self::Mul
                | Self::Div
                | Self::Rem
                | Self::Eq
                | Self::Ne
                | Self::Lt
                | Self::Le
                | Self::Gt
                | Self::Ge
        )
    }

    #[must_use]
    pub const fn effects(self) -> super::BuiltinEffects {
        super::BuiltinEffects {
            may_empty: matches!(self, Self::Empty),
            may_stderr: matches!(self, Self::Debug | Self::Stderr),
            may_add_input: matches!(self, Self::Input),
            may_halt: matches!(self, Self::Halt | Self::HaltError),
        }
    }
}
