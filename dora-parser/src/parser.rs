use std::cell::RefCell;
use std::mem;
use std::sync::Arc;

use crate::ast;
use crate::ast::*;
use crate::builder::Builder;
use crate::error::{ParseError, ParseErrorAndPos};

use crate::interner::*;

use crate::lexer::position::{Position, Span};
use crate::lexer::reader::Reader;
use crate::lexer::token::*;
use crate::lexer::*;

pub struct Parser<'a> {
    lexer: Lexer,
    token: Token,
    id_generator: NodeIdGenerator,
    interner: &'a mut Interner,
    param_idx: u32,
    in_class_or_module: bool,
    last_end: Option<u32>,
}

type ExprResult = Result<Box<Expr>, ParseErrorAndPos>;
type StmtResult = Result<Box<Stmt>, ParseErrorAndPos>;
type StmtOrExprResult = Result<StmtOrExpr, ParseErrorAndPos>;

enum StmtOrExpr {
    Stmt(Box<Stmt>),
    Expr(Box<Expr>),
}

impl<'a> Parser<'a> {
    pub fn from_string(code: &'static str, interner: &'a mut Interner) -> Parser<'a> {
        let reader = Reader::from_string(code);
        Parser::common_init(reader, interner)
    }

    pub fn from_shared_string(content: Arc<String>, interner: &'a mut Interner) -> Parser<'a> {
        let reader = Reader::from_shared_string(content);
        Parser::common_init(reader, interner)
    }

    fn common_init(reader: Reader, interner: &'a mut Interner) -> Parser<'a> {
        let token = Token::new(TokenKind::End, Position::new(1, 1), Span::invalid());
        let lexer = Lexer::new(reader);

        let parser = Parser {
            lexer,
            token,
            id_generator: NodeIdGenerator::new(),
            interner,
            param_idx: 0,
            in_class_or_module: false,
            last_end: Some(0),
        };

        parser
    }

    fn generate_id(&mut self) -> NodeId {
        self.id_generator.next()
    }

    pub fn parse(mut self) -> Result<ast::File, ParseErrorAndPos> {
        self.init()?;
        let mut elements = vec![];

        while !self.token.is_eof() {
            elements.push(self.parse_top_level_element()?);
        }

        let ast_file = ast::File { elements };

        Ok(ast_file)
    }

    fn init(&mut self) -> Result<(), ParseErrorAndPos> {
        self.advance_token()?;

        Ok(())
    }

    fn parse_top_level_element(&mut self) -> Result<Elem, ParseErrorAndPos> {
        let modifiers = self.parse_annotation_usages()?;

        match self.token.kind {
            TokenKind::Fn => {
                self.restrict_modifiers(
                    &modifiers,
                    &[
                        Modifier::Internal,
                        Modifier::OptimizeImmediately,
                        Modifier::Test,
                        Modifier::Pub,
                    ],
                )?;
                let fct = self.parse_function(&modifiers)?;
                Ok(Elem::Function(Arc::new(fct)))
            }

            TokenKind::Class => {
                self.restrict_modifiers(
                    &modifiers,
                    &[
                        Modifier::Abstract,
                        Modifier::Open,
                        Modifier::Internal,
                        Modifier::Pub,
                    ],
                )?;
                let class = self.parse_class(&modifiers)?;
                Ok(Elem::Class(Arc::new(class)))
            }

            TokenKind::Class2 => {
                self.restrict_modifiers(&modifiers, &[Modifier::Internal, Modifier::Pub])?;
                let class = self.parse_class2(&modifiers)?;
                Ok(Elem::Class(Arc::new(class)))
            }

            TokenKind::Struct => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub, Modifier::Internal])?;
                let struc = self.parse_struct(&modifiers)?;
                Ok(Elem::Struct(Arc::new(struc)))
            }

            TokenKind::Trait => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let trait_ = self.parse_trait(&modifiers)?;
                Ok(Elem::Trait(Arc::new(trait_)))
            }

            TokenKind::Impl => {
                self.ban_modifiers(&modifiers)?;
                let impl_ = self.parse_impl()?;
                Ok(Elem::Impl(Arc::new(impl_)))
            }

            TokenKind::Annotation => {
                let annotation = self.parse_annotation(&modifiers)?;
                Ok(Elem::Annotation(Arc::new(annotation)))
            }

            TokenKind::Alias => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let alias = self.parse_alias(&modifiers)?;
                Ok(Elem::Alias(Arc::new(alias)))
            }

            TokenKind::Let | TokenKind::Var => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let global = self.parse_global(&modifiers)?;
                Ok(Elem::Global(Arc::new(global)))
            }

            TokenKind::Const => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let const_ = self.parse_const(&modifiers)?;
                Ok(Elem::Const(Arc::new(const_)))
            }

            TokenKind::Enum => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let enum_ = self.parse_enum(&modifiers)?;
                Ok(Elem::Enum(Arc::new(enum_)))
            }

            TokenKind::Mod => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let module = self.parse_module(&modifiers)?;
                Ok(Elem::Module(Arc::new(module)))
            }

            TokenKind::Use => {
                self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;
                let use_stmt = self.parse_use()?;
                Ok(Elem::Use(Arc::new(use_stmt)))
            }

            _ => {
                let msg = ParseError::ExpectedTopLevelElement(self.token.name());
                return Err(ParseErrorAndPos::new(self.token.position, msg));
            }
        }
    }

    fn parse_use(&mut self) -> Result<Use, ParseErrorAndPos> {
        self.expect_token(TokenKind::Use)?;
        let use_declaration = self.parse_use_inner()?;
        self.expect_semicolon()?;

        Ok(use_declaration)
    }

    fn parse_use_inner(&mut self) -> Result<Use, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let mut path = Vec::new();
        let mut allow_brace = false;

        loop {
            if self.token.is(TokenKind::LBrace) {
                allow_brace = true;
                break;
            }

            let component = self.parse_use_path_component()?;
            path.push(component);

            if self.token.is(TokenKind::ColonColon) {
                self.expect_token(TokenKind::ColonColon)?;
            } else {
                break;
            }
        }

        let target = if allow_brace && self.token.is(TokenKind::LBrace) {
            self.parse_use_brace()?
        } else if self.token.is(TokenKind::As) {
            UseTargetDescriptor::As(self.parse_use_as()?)
        } else {
            UseTargetDescriptor::Default
        };

        let span = self.span_from(start);

        Ok(Use {
            id: self.generate_id(),
            pos,
            span,
            common_path: path,
            target,
        })
    }

    fn parse_use_as(&mut self) -> Result<UseTargetName, ParseErrorAndPos> {
        self.expect_token(TokenKind::As)?;

        let pos = self.token.position;
        let start = self.token.span.start();

        let name = if self.token.is(TokenKind::Underscore) {
            self.expect_token(TokenKind::Underscore)?;
            None
        } else {
            Some(self.expect_identifier()?)
        };

        let span = self.span_from(start);
        Ok(UseTargetName { pos, span, name })
    }

    fn parse_use_path_component(&mut self) -> Result<UsePathComponent, ParseErrorAndPos> {
        let pos = self.token.position;
        let start = self.token.span.start();

        let value = if self.token.is(TokenKind::This) {
            self.expect_token(TokenKind::This)?;
            UsePathComponentValue::This
        } else if self.token.is(TokenKind::Package) {
            self.expect_token(TokenKind::Package)?;
            UsePathComponentValue::Package
        } else if self.token.is(TokenKind::Super) {
            self.expect_token(TokenKind::Super)?;
            UsePathComponentValue::Super
        } else {
            let name = self.expect_identifier()?;
            UsePathComponentValue::Name(name)
        };

        let span = self.span_from(start);

        Ok(UsePathComponent { pos, span, value })
    }

    fn parse_use_brace(&mut self) -> Result<UseTargetDescriptor, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::LBrace)?.position;

        let targets = self.parse_list(TokenKind::Comma, TokenKind::RBrace, |p| {
            let use_decl = p.parse_use_inner()?;
            Ok(Arc::new(use_decl))
        })?;

        let span = self.span_from(start);

        Ok(UseTargetDescriptor::Group(UseTargetGroup {
            pos,
            span,
            targets,
        }))
    }

    fn parse_enum(&mut self, modifiers: &Modifiers) -> Result<Enum, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Enum)?.position;
        let name = self.expect_identifier()?;
        let type_params = self.parse_type_params()?;

        self.expect_token(TokenKind::LBrace)?;
        let variants = self.parse_list(TokenKind::Comma, TokenKind::RBrace, |p| {
            p.parse_enum_variant()
        })?;
        let span = self.span_from(start);

        Ok(Enum {
            id: self.generate_id(),
            pos,
            span,
            name,
            type_params,
            variants,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_module(&mut self, modifiers: &Modifiers) -> Result<Module, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Mod)?.position;
        let name = self.expect_identifier()?;

        let elements = if self.token.is(TokenKind::LBrace) {
            self.expect_token(TokenKind::LBrace)?;

            let mut elements = Vec::new();

            while !self.token.is(TokenKind::RBrace) && !self.token.is_eof() {
                elements.push(self.parse_top_level_element()?);
            }

            self.expect_token(TokenKind::RBrace)?;
            Some(elements)
        } else {
            self.expect_token(TokenKind::Semicolon)?;
            None
        };

        let span = self.span_from(start);

        Ok(Module {
            id: self.generate_id(),
            pos,
            span,
            name,
            elements,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_enum_variant(&mut self) -> Result<EnumVariant, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let name = self.expect_identifier()?;

        let types = if self.token.is(TokenKind::LParen) {
            self.advance_token()?;
            Some(self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| p.parse_type())?)
        } else {
            None
        };

        let span = self.span_from(start);

        Ok(EnumVariant {
            id: self.generate_id(),
            pos,
            span,
            name,
            types,
        })
    }

    fn parse_const(&mut self, modifiers: &Modifiers) -> Result<Const, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Const)?.position;
        let name = self.expect_identifier()?;
        self.expect_token(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        self.expect_token(TokenKind::Eq)?;
        let expr = self.parse_expression()?;
        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Const {
            id: self.generate_id(),
            pos,
            span,
            name,
            data_type: ty,
            expr,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_impl(&mut self) -> Result<Impl, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Impl)?.position;
        let type_params = self.parse_type_params()?;

        let type_name = self.parse_type()?;

        let (class_type, trait_type) = if self.token.is(TokenKind::For) {
            self.advance_token()?;
            let class_type = self.parse_type()?;

            (class_type, Some(type_name))
        } else {
            (type_name, None)
        };

        self.expect_token(TokenKind::LBrace)?;

        let mut methods = Vec::new();

        while !self.token.is(TokenKind::RBrace) {
            let modifiers = self.parse_annotation_usages()?;
            let mods = &[Modifier::Static, Modifier::Internal, Modifier::Pub];
            self.restrict_modifiers(&modifiers, mods)?;

            let method = self.parse_function(&modifiers)?;
            methods.push(Arc::new(method));
        }

        self.expect_token(TokenKind::RBrace)?;
        let span = self.span_from(start);

        Ok(Impl {
            id: self.generate_id(),
            pos,
            span,
            type_params,
            trait_type,
            extended_type: class_type,
            methods,
        })
    }

    fn parse_global(&mut self, modifiers: &Modifiers) -> Result<Global, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let mutable = self.token.is(TokenKind::Var);

        self.advance_token()?;
        let name = self.expect_identifier()?;

        self.expect_token(TokenKind::Colon)?;
        let data_type = self.parse_type()?;

        let expr = if self.token.is(TokenKind::Eq) {
            self.advance_token()?;
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.expect_semicolon()?;
        let span = self.span_from(start);

        let mut global = Global {
            id: self.generate_id(),
            name,
            pos,
            span,
            data_type,
            mutable: mutable,
            initializer: None,
            is_pub: modifiers.contains(Modifier::Pub),
        };

        if let Some(expr) = expr {
            let initializer = self.generate_global_initializer(&global, expr);
            global.initializer = Some(Arc::new(initializer));
        }

        Ok(global)
    }

    fn parse_trait(&mut self, modifiers: &Modifiers) -> Result<Trait, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Trait)?.position;
        let ident = self.expect_identifier()?;
        let type_params = self.parse_type_params()?;

        self.expect_token(TokenKind::LBrace)?;

        let mut methods = Vec::new();

        while !self.token.is(TokenKind::RBrace) {
            let modifiers = self.parse_annotation_usages()?;
            let mods = &[Modifier::Static];
            self.restrict_modifiers(&modifiers, mods)?;

            let method = self.parse_function(&modifiers)?;
            methods.push(Arc::new(method));
        }

        self.expect_token(TokenKind::RBrace)?;
        let span = self.span_from(start);

        Ok(Trait {
            id: self.generate_id(),
            name: ident,
            type_params,
            pos,
            span,
            methods,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_struct(&mut self, modifiers: &Modifiers) -> Result<Struct, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Struct)?.position;
        let ident = self.expect_identifier()?;
        let type_params = self.parse_type_params()?;

        let fields = if self.token.is(TokenKind::LParen) {
            self.expect_token(TokenKind::LParen)?;
            self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
                p.parse_struct_field()
            })?
        } else if self.token.is(TokenKind::LBrace) {
            self.expect_token(TokenKind::LBrace)?;
            self.parse_list(TokenKind::Comma, TokenKind::RBrace, |p| {
                p.parse_struct_field()
            })?
        } else {
            Vec::new()
        };

        let span = self.span_from(start);

        Ok(Struct {
            id: self.generate_id(),
            name: ident,
            pos,
            span,
            fields,
            is_pub: modifiers.contains(Modifier::Pub),
            internal: modifiers.contains(Modifier::Internal),
            type_params,
        })
    }

    fn parse_struct_field(&mut self) -> Result<StructField, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;

        let modifiers = self.parse_annotation_usages()?;
        let mods = &[Modifier::Pub];
        self.restrict_modifiers(&modifiers, mods)?;

        let ident = self.expect_identifier()?;

        self.expect_token(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        let span = self.span_from(start);

        Ok(StructField {
            id: self.generate_id(),
            name: ident,
            pos,
            span,
            data_type: ty,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_class(&mut self, modifiers: &Modifiers) -> Result<Class, ParseErrorAndPos> {
        let start = self.token.span.start();
        let is_open = modifiers.contains(Modifier::Open);
        let internal = modifiers.contains(Modifier::Internal);
        let is_abstract = modifiers.contains(Modifier::Abstract);
        let is_pub = modifiers.contains(Modifier::Pub);

        let pos = self.expect_token(TokenKind::Class)?.position;

        let ident = self.expect_identifier()?;
        let type_params = self.parse_type_params()?;

        let mut cls = Class {
            id: self.generate_id(),
            name: ident,
            pos,
            span: Span::invalid(),
            is_open,
            internal,
            is_abstract,
            is_pub,
            has_constructor: false,
            parent_class: None,
            constructor: None,
            fields: Vec::new(),
            methods: Vec::new(),
            initializers: Vec::new(),
            type_params,
        };

        self.in_class_or_module = true;
        let ctor_params = self.parse_constructor(&mut cls)?;

        cls.parent_class = self.parse_class_parent()?;

        self.parse_class_body(&mut cls)?;
        let span = self.span_from(start);

        let constructor = self.generate_constructor(&mut cls, ctor_params);
        cls.constructor = Some(Arc::new(constructor));
        cls.span = span;
        self.in_class_or_module = false;

        Ok(cls)
    }

    fn parse_class_parent(&mut self) -> Result<Option<ParentClass>, ParseErrorAndPos> {
        if self.token.is(TokenKind::Colon) {
            self.advance_token()?;

            let start = self.token.span.start();
            let pos = self.token.position;
            let parent_ty = self.parse_type()?;
            let params = self.parse_parent_class_params()?;
            let span = self.span_from(start);

            Ok(Some(ParentClass::new(pos, span, parent_ty, params)))
        } else {
            Ok(None)
        }
    }

    fn parse_class2(&mut self, modifiers: &Modifiers) -> Result<Class, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Class2)?.position;

        let ident = self.expect_identifier()?;
        let type_params = self.parse_type_params()?;

        let fields = if self.token.is(TokenKind::LParen) {
            self.expect_token(TokenKind::LParen)?;
            self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
                p.parse_class2_field()
            })?
        } else if self.token.is(TokenKind::LBrace) {
            self.expect_token(TokenKind::LBrace)?;
            self.parse_list(TokenKind::Comma, TokenKind::RBrace, |p| {
                p.parse_class2_field()
            })?
        } else {
            Vec::new()
        };

        let span = self.span_from(start);

        Ok(Class {
            id: self.generate_id(),
            name: ident,
            pos,
            span,
            is_open: false,
            internal: modifiers.contains(Modifier::Internal),
            is_abstract: false,
            is_pub: modifiers.contains(Modifier::Pub),
            has_constructor: false,
            parent_class: None,
            constructor: None,
            fields,
            methods: Vec::new(),
            initializers: Vec::new(),
            type_params,
        })
    }

    fn parse_class2_field(&mut self) -> Result<Field, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;

        let modifiers = self.parse_annotation_usages()?;
        let mods = &[Modifier::Pub];
        self.restrict_modifiers(&modifiers, mods)?;

        let name = self.expect_identifier()?;

        self.expect_token(TokenKind::Colon)?;
        let data_type = self.parse_type()?;
        let span = self.span_from(start);

        Ok(Field {
            id: self.generate_id(),
            name,
            pos,
            span,
            data_type,
            primary_ctor: false,
            expr: None,
            mutable: true,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_annotation(&mut self, modifiers: &Modifiers) -> Result<Annotation, ParseErrorAndPos> {
        let internal = modifiers.contains(Modifier::Internal);

        let pos = self.expect_token(TokenKind::Annotation)?.position;
        let ident = self.expect_identifier()?;
        let internal = if internal {
            Modifier::find(&self.interner.str(ident))
        } else {
            None
        };
        let type_params = self.parse_type_params()?;
        let term_params = self.parse_annotation_params()?;
        let annotation = Annotation {
            id: self.generate_id(),
            name: ident,
            pos: pos,
            // use method argument after signature has been adapted
            annotation_usages: AnnotationUsages::new(),
            internal: internal,
            type_params: type_params,
            term_params: term_params,
        };

        Ok(annotation)
    }

    fn parse_annotation_params(
        &mut self,
    ) -> Result<Option<Vec<AnnotationParam>>, ParseErrorAndPos> {
        if !self.token.is(TokenKind::LParen) {
            return Ok(None);
        }

        self.expect_token(TokenKind::LParen)?;

        let params = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
            p.parse_annotation_param()
        })?;

        Ok(Some(params))
    }

    fn parse_annotation_param(&mut self) -> Result<AnnotationParam, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let name = self.expect_identifier()?;

        self.expect_token(TokenKind::Colon)?;
        let data_type = self.parse_type()?;

        let span = self.span_from(start);

        Ok(AnnotationParam {
            name,
            pos,
            span,
            data_type,
        })
    }

    fn parse_alias(&mut self, modifiers: &Modifiers) -> Result<Alias, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Alias)?.position;
        let name = self.expect_identifier()?;
        self.expect_token(TokenKind::Eq)?;
        let ty = self.parse_type()?;
        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Alias {
            id: self.generate_id(),
            pos,
            name,
            span,
            ty,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_type_params(&mut self) -> Result<Option<Vec<TypeParam>>, ParseErrorAndPos> {
        if self.token.is(TokenKind::LBracket) {
            self.advance_token()?;
            let params = self.parse_list(TokenKind::Comma, TokenKind::RBracket, |p| {
                p.parse_type_param()
            })?;

            Ok(Some(params))
        } else {
            Ok(None)
        }
    }

    fn parse_type_param(&mut self) -> Result<TypeParam, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let name = self.expect_identifier()?;

        let bounds = if self.token.is(TokenKind::Colon) {
            self.advance_token()?;

            let mut bounds = Vec::new();

            loop {
                bounds.push(self.parse_type()?);

                if self.token.is(TokenKind::Add) {
                    self.advance_token()?;
                } else {
                    break;
                }
            }

            bounds
        } else {
            Vec::new()
        };

        let span = self.span_from(start);

        Ok(TypeParam {
            name,
            span,
            pos,
            bounds,
        })
    }

    fn parse_parent_class_params(&mut self) -> Result<Vec<Box<Expr>>, ParseErrorAndPos> {
        if !self.token.is(TokenKind::LParen) {
            return Ok(Vec::new());
        }

        self.expect_token(TokenKind::LParen)?;

        let params = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
            p.parse_expression()
        })?;

        Ok(params)
    }

    fn parse_constructor(
        &mut self,
        cls: &mut Class,
    ) -> Result<Vec<ConstructorParam>, ParseErrorAndPos> {
        if !self.token.is(TokenKind::LParen) {
            return Ok(Vec::new());
        }

        self.expect_token(TokenKind::LParen)?;
        cls.has_constructor = true;

        let params = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
            p.parse_constructor_param(cls)
        })?;

        Ok(params)
    }

    fn parse_constructor_param(
        &mut self,
        cls: &mut Class,
    ) -> Result<ConstructorParam, ParseErrorAndPos> {
        let start = self.token.span.start();
        let field = self.token.is(TokenKind::Var) || self.token.is(TokenKind::Let);
        let mutable = self.token.is(TokenKind::Var);

        // consume var and let
        if field {
            self.advance_token()?;
        }

        let pos = self.token.position;
        let name = self.expect_identifier()?;

        self.expect_token(TokenKind::Colon)?;
        let data_type = self.parse_type()?;

        let variadic = if self.token.is(TokenKind::DotDotDot) {
            self.advance_token()?;
            true
        } else {
            false
        };

        let span = self.span_from(start);

        if field {
            cls.fields.push(Field {
                id: self.generate_id(),
                name,
                pos,
                span,
                data_type: data_type.clone(),
                primary_ctor: true,
                expr: None,
                mutable,
                is_pub: true,
            })
        }

        Ok(ConstructorParam {
            name,
            pos,
            span,
            data_type,
            variadic,
            field,
            mutable: mutable,
        })
    }

    fn parse_class_body(&mut self, cls: &mut Class) -> Result<(), ParseErrorAndPos> {
        if !self.token.is(TokenKind::LBrace) {
            return Ok(());
        }

        self.advance_token()?;

        while !self.token.is(TokenKind::RBrace) {
            let modifiers = self.parse_annotation_usages()?;

            match self.token.kind {
                TokenKind::Fn => {
                    let mods = &[
                        Modifier::Abstract,
                        Modifier::Internal,
                        Modifier::Open,
                        Modifier::Override,
                        Modifier::Final,
                        Modifier::Pub,
                        Modifier::Static,
                    ];
                    self.restrict_modifiers(&modifiers, mods)?;

                    let fct = self.parse_function(&modifiers)?;
                    cls.methods.push(Arc::new(fct));
                }

                TokenKind::Var | TokenKind::Let => {
                    self.restrict_modifiers(&modifiers, &[Modifier::Pub])?;

                    let field = self.parse_field(&modifiers)?;
                    cls.fields.push(field);
                }

                _ => {
                    let initializer = self.parse_statement()?;
                    cls.initializers.push(initializer);
                }
            }
        }

        self.advance_token()?;
        Ok(())
    }

    fn parse_annotation_usages(&mut self) -> Result<Modifiers, ParseErrorAndPos> {
        let mut modifiers = Modifiers::new();
        loop {
            if !self.token.is(TokenKind::At) {
                break;
            }
            self.advance_token()?;
            let ident = self.expect_identifier()?;
            let modifier = match self.interner.str(ident).as_str() {
                "abstract" => Modifier::Abstract,
                "open" => Modifier::Open,
                "override" => Modifier::Override,
                "final" => Modifier::Final,
                "internal" => Modifier::Internal,
                "pub" => Modifier::Pub,
                "static" => Modifier::Static,
                "Test" => Modifier::Test,
                "optimizeImmediately" => Modifier::OptimizeImmediately,
                annotation => {
                    return Err(ParseErrorAndPos::new(
                        self.token.position,
                        ParseError::UnknownAnnotation(annotation.into()),
                    ));
                }
            };

            if modifiers.contains(modifier) {
                return Err(ParseErrorAndPos::new(
                    self.token.position,
                    ParseError::RedundantAnnotation(modifier.name().into()),
                ));
            }

            modifiers.add(modifier, self.token.position, self.token.span);
        }

        Ok(modifiers)
    }

    fn ban_modifiers(&mut self, modifiers: &Modifiers) -> Result<(), ParseErrorAndPos> {
        self.restrict_modifiers(modifiers, &[])
    }

    fn restrict_modifiers(
        &mut self,
        modifiers: &Modifiers,
        restrict: &[Modifier],
    ) -> Result<(), ParseErrorAndPos> {
        for modifier in modifiers.iter() {
            if !restrict.contains(&modifier.value) {
                return Err(ParseErrorAndPos::new(
                    modifier.pos,
                    ParseError::MisplacedAnnotation(modifier.value.name().into()),
                ));
            }
        }

        Ok(())
    }

    fn parse_field(&mut self, modifiers: &Modifiers) -> Result<Field, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let mutable = if self.token.is(TokenKind::Var) {
            self.expect_token(TokenKind::Var)?;

            true
        } else {
            self.expect_token(TokenKind::Let)?;

            false
        };

        let name = self.expect_identifier()?;
        self.expect_token(TokenKind::Colon)?;
        let data_type = self.parse_type()?;

        let expr = if self.token.is(TokenKind::Eq) {
            self.expect_token(TokenKind::Eq)?;
            Some(self.parse_expression()?)
        } else {
            None
        };

        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Field {
            id: self.generate_id(),
            name,
            pos,
            span,
            data_type,
            primary_ctor: false,
            expr,
            mutable: mutable,
            is_pub: modifiers.contains(Modifier::Pub),
        })
    }

    fn parse_function(&mut self, modifiers: &Modifiers) -> Result<Function, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Fn)?.position;
        let ident = self.expect_identifier()?;
        let type_params = self.parse_type_params()?;
        let params = self.parse_function_params()?;
        let return_type = self.parse_function_type()?;
        let block = self.parse_function_block()?;
        let span = self.span_from(start);

        Ok(Function {
            id: self.generate_id(),
            kind: FunctionKind::Function,
            name: ident,
            pos,
            span,
            method: self.in_class_or_module,
            is_open: modifiers.contains(Modifier::Open),
            is_override: modifiers.contains(Modifier::Override),
            is_final: modifiers.contains(Modifier::Final),
            is_optimize_immediately: modifiers.contains(Modifier::OptimizeImmediately),
            is_pub: modifiers.contains(Modifier::Pub),
            is_static: modifiers.contains(Modifier::Static),
            internal: modifiers.contains(Modifier::Internal),
            is_abstract: modifiers.contains(Modifier::Abstract),
            is_constructor: false,
            is_test: modifiers.contains(Modifier::Test),
            params,
            return_type,
            block,
            type_params,
        })
    }

    fn parse_function_params(&mut self) -> Result<Vec<Param>, ParseErrorAndPos> {
        self.expect_token(TokenKind::LParen)?;
        self.param_idx = 0;

        let params = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
            p.param_idx += 1;

            p.parse_function_param()
        })?;

        Ok(params)
    }

    fn parse_list<F, R>(
        &mut self,
        sep: TokenKind,
        stop: TokenKind,
        mut parse: F,
    ) -> Result<Vec<R>, ParseErrorAndPos>
    where
        F: FnMut(&mut Parser) -> Result<R, ParseErrorAndPos>,
    {
        let mut data = vec![];
        let mut comma = true;

        while !self.token.is(stop.clone()) && !self.token.is_eof() {
            if !comma {
                return Err(ParseErrorAndPos::new(
                    self.token.position,
                    ParseError::ExpectedToken(sep.name().into(), self.token.name()),
                ));
            }

            let entry = parse(self)?;
            data.push(entry);

            comma = self.token.is(sep.clone());
            if comma {
                self.advance_token()?;
            }
        }

        self.expect_token(stop)?;

        Ok(data)
    }

    fn parse_function_param(&mut self) -> Result<Param, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;

        let name = self.expect_identifier()?;

        self.expect_token(TokenKind::Colon)?;

        let data_type = self.parse_type()?;

        let variadic = if self.token.is(TokenKind::DotDotDot) {
            self.advance_token()?;
            true
        } else {
            false
        };

        let span = self.span_from(start);

        Ok(Param {
            id: self.generate_id(),
            idx: self.param_idx - 1,
            variadic,
            name,
            pos,
            span,
            data_type,
        })
    }

    fn parse_function_type(&mut self) -> Result<Option<Type>, ParseErrorAndPos> {
        if self.token.is(TokenKind::Colon) {
            self.advance_token()?;
            let ty = self.parse_type()?;

            Ok(Some(ty))
        } else {
            Ok(None)
        }
    }

    fn parse_function_block(&mut self) -> Result<Option<Box<ExprBlockType>>, ParseErrorAndPos> {
        if self.token.is(TokenKind::Semicolon) {
            self.advance_token()?;

            Ok(None)
        } else if self.token.is(TokenKind::Eq) {
            let expr = self.parse_function_block_expression()?;

            Ok(Some(expr))
        } else {
            let block = self.parse_block()?;

            if let Expr::Block(block_type) = *block {
                Ok(Some(Box::new(block_type)))
            } else {
                unreachable!()
            }
        }
    }

    fn parse_function_block_expression(&mut self) -> Result<Box<ExprBlockType>, ParseErrorAndPos> {
        self.expect_token(TokenKind::Eq)?;

        match self.token.kind {
            TokenKind::Return => {
                let stmt = self.parse_return()?;
                Ok(Box::new(ExprBlockType {
                    id: self.generate_id(),
                    pos: stmt.pos(),
                    span: stmt.span(),
                    stmts: vec![stmt],
                    expr: None,
                }))
            }

            _ => {
                let expr = self.parse_expression()?;
                self.expect_token(TokenKind::Semicolon)?;
                Ok(Box::new(ExprBlockType {
                    id: self.generate_id(),
                    pos: expr.pos(),
                    span: expr.span(),
                    stmts: Vec::new(),
                    expr: Some(expr),
                }))
            }
        }
    }

    fn parse_type(&mut self) -> Result<Type, ParseErrorAndPos> {
        match self.token.kind {
            TokenKind::CapitalThis => {
                let pos = self.token.position;
                let span = self.token.span;
                self.advance_token()?;
                Ok(Type::create_self(self.generate_id(), pos, span))
            }

            TokenKind::Identifier(_) => {
                let pos = self.token.position;
                let start = self.token.span.start();
                let path = self.parse_path()?;

                let params = if self.token.is(TokenKind::LBracket) {
                    self.advance_token()?;
                    self.parse_list(TokenKind::Comma, TokenKind::RBracket, |p| {
                        Ok(Box::new(p.parse_type()?))
                    })?
                } else {
                    Vec::new()
                };

                let span = self.span_from(start);
                Ok(Type::create_basic(
                    self.generate_id(),
                    pos,
                    span,
                    path,
                    params,
                ))
            }

            TokenKind::LParen => {
                let start = self.token.span.start();
                let token = self.advance_token()?;
                let subtypes = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
                    let ty = p.parse_type()?;

                    Ok(Box::new(ty))
                })?;

                if self.token.is(TokenKind::Colon) {
                    self.advance_token()?;
                    let ret = Box::new(self.parse_type()?);
                    let span = self.span_from(start);

                    Ok(Type::create_fct(
                        self.generate_id(),
                        token.position,
                        span,
                        subtypes,
                        ret,
                    ))
                } else {
                    let span = self.span_from(start);
                    Ok(Type::create_tuple(
                        self.generate_id(),
                        token.position,
                        span,
                        subtypes,
                    ))
                }
            }

            _ => Err(ParseErrorAndPos::new(
                self.token.position,
                ParseError::ExpectedType(self.token.name()),
            )),
        }
    }

    fn parse_path(&mut self) -> Result<Path, ParseErrorAndPos> {
        let pos = self.token.position;
        let start = self.token.span.start();
        let name = self.expect_identifier()?;
        let mut names = vec![name];

        while self.token.is(TokenKind::ColonColon) {
            self.advance_token()?;
            let name = self.expect_identifier()?;
            names.push(name);
        }

        let span = self.span_from(start);

        Ok(Path {
            id: self.generate_id(),
            pos,
            span,
            names,
        })
    }

    fn parse_statement(&mut self) -> StmtResult {
        let stmt_or_expr = self.parse_statement_or_expression()?;

        match stmt_or_expr {
            StmtOrExpr::Stmt(stmt) => Ok(stmt),
            StmtOrExpr::Expr(expr) => {
                if expr.needs_semicolon() {
                    Err(self.expect_semicolon().unwrap_err())
                } else {
                    Ok(Box::new(Stmt::create_expr(
                        self.generate_id(),
                        expr.pos(),
                        expr.span(),
                        expr,
                    )))
                }
            }
        }
    }

    fn parse_let(&mut self) -> StmtResult {
        let start = self.token.span.start();
        let mutable = if self.token.is(TokenKind::Let) {
            false
        } else if self.token.is(TokenKind::Var) {
            true
        } else {
            panic!("let or var expected")
        };

        let pos = self.advance_token()?.position;
        let pattern = self.parse_let_pattern()?;
        let data_type = self.parse_var_type()?;
        let expr = self.parse_var_assignment()?;

        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Box::new(Stmt::create_let(
            self.generate_id(),
            pos,
            span,
            pattern,
            mutable,
            data_type,
            expr,
        )))
    }

    fn parse_let_pattern(&mut self) -> Result<Box<LetPattern>, ParseErrorAndPos> {
        if self.token.is(TokenKind::LParen) {
            let pos = self.token.position;
            let start = self.token.span.start();
            self.advance_token()?;

            let parts = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
                p.parse_let_pattern()
            })?;

            let span = self.span_from(start);

            Ok(Box::new(LetPattern::Tuple(LetTupleType {
                id: self.generate_id(),
                pos,
                span,
                parts,
            })))
        } else if self.token.is(TokenKind::Underscore) {
            let pos = self.token.position;
            let span = self.token.span;
            self.advance_token()?;

            Ok(Box::new(LetPattern::Underscore(LetUnderscoreType {
                id: self.generate_id(),
                pos,
                span,
            })))
        } else {
            let start = self.token.span.start();
            let mutable = if self.token.is(TokenKind::Mut) {
                self.advance_token()?;
                true
            } else {
                false
            };
            let pos = self.token.position;
            let name = self.expect_identifier()?;
            let span = self.span_from(start);

            Ok(Box::new(LetPattern::Ident(LetIdentType {
                id: self.generate_id(),
                pos,
                span,
                mutable,
                name,
            })))
        }
    }

    fn parse_var_type(&mut self) -> Result<Option<Type>, ParseErrorAndPos> {
        if self.token.is(TokenKind::Colon) {
            self.advance_token()?;

            Ok(Some(self.parse_type()?))
        } else {
            Ok(None)
        }
    }

    fn parse_var_assignment(&mut self) -> Result<Option<Box<Expr>>, ParseErrorAndPos> {
        if self.token.is(TokenKind::Eq) {
            self.expect_token(TokenKind::Eq)?;
            let expr = self.parse_expression()?;

            Ok(Some(expr))
        } else {
            Ok(None)
        }
    }

    fn parse_block_stmt(&mut self) -> StmtResult {
        let block = self.parse_block()?;
        Ok(Box::new(Stmt::create_expr(
            self.generate_id(),
            block.pos(),
            block.span(),
            block,
        )))
    }

    fn parse_block(&mut self) -> ExprResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::LBrace)?.position;
        let mut stmts = vec![];
        let mut expr = None;

        while !self.token.is(TokenKind::RBrace) && !self.token.is_eof() {
            let stmt_or_expr = self.parse_statement_or_expression()?;

            match stmt_or_expr {
                StmtOrExpr::Stmt(stmt) => stmts.push(stmt),
                StmtOrExpr::Expr(curr_expr) => {
                    if curr_expr.needs_semicolon() {
                        expr = Some(curr_expr);
                        break;
                    } else if !self.token.is(TokenKind::RBrace) {
                        stmts.push(Box::new(Stmt::create_expr(
                            self.generate_id(),
                            curr_expr.pos(),
                            curr_expr.span(),
                            curr_expr,
                        )));
                    } else {
                        expr = Some(curr_expr);
                    }
                }
            }
        }

        self.expect_token(TokenKind::RBrace)?;
        let span = self.span_from(start);

        Ok(Box::new(Expr::create_block(
            self.generate_id(),
            pos,
            span,
            stmts,
            expr,
        )))
    }

    fn parse_statement_or_expression(&mut self) -> StmtOrExprResult {
        match self.token.kind {
            TokenKind::Let | TokenKind::Var => Ok(StmtOrExpr::Stmt(self.parse_let()?)),
            TokenKind::While => Ok(StmtOrExpr::Stmt(self.parse_while()?)),
            TokenKind::Break => Ok(StmtOrExpr::Stmt(self.parse_break()?)),
            TokenKind::Continue => Ok(StmtOrExpr::Stmt(self.parse_continue()?)),
            TokenKind::Return => Ok(StmtOrExpr::Stmt(self.parse_return()?)),
            TokenKind::Else => Err(ParseErrorAndPos::new(
                self.token.position,
                ParseError::MisplacedElse,
            )),
            TokenKind::For => Ok(StmtOrExpr::Stmt(self.parse_for()?)),
            _ => {
                let expr = self.parse_expression()?;

                if self.token.is(TokenKind::Semicolon) {
                    self.expect_token(TokenKind::Semicolon)?;
                    let span = self.span_from(expr.span().start());

                    Ok(StmtOrExpr::Stmt(Box::new(Stmt::create_expr(
                        self.generate_id(),
                        expr.pos(),
                        span,
                        expr,
                    ))))
                } else {
                    Ok(StmtOrExpr::Expr(expr))
                }
            }
        }
    }

    fn parse_if(&mut self) -> ExprResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::If)?.position;

        let cond = self.parse_expression()?;

        let then_block = self.parse_block()?;

        let else_block = if self.token.is(TokenKind::Else) {
            self.advance_token()?;

            if self.token.is(TokenKind::If) {
                Some(self.parse_if()?)
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };

        let span = self.span_from(start);

        Ok(Box::new(Expr::create_if(
            self.generate_id(),
            pos,
            span,
            cond,
            then_block,
            else_block,
        )))
    }

    fn parse_match(&mut self) -> ExprResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Match)?.position;

        let expr = self.parse_expression()?;
        let mut cases = Vec::new();
        let mut comma = true;

        self.expect_token(TokenKind::LBrace)?;

        while !self.token.is(TokenKind::RBrace) && !self.token.is_eof() {
            if !comma {
                return Err(ParseErrorAndPos::new(
                    self.token.position,
                    ParseError::ExpectedToken(TokenKind::Comma.name().into(), self.token.name()),
                ));
            }

            let case = self.parse_match_case()?;
            cases.push(case);

            comma = self.token.is(TokenKind::Comma);

            if comma {
                self.advance_token()?;
            }
        }

        self.expect_token(TokenKind::RBrace)?;
        let span = self.span_from(start);

        Ok(Box::new(Expr::create_match(
            self.generate_id(),
            pos,
            span,
            expr,
            cases,
        )))
    }

    fn parse_match_case(&mut self) -> Result<MatchCaseType, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;
        let mut patterns = Vec::new();
        patterns.push(self.parse_match_pattern()?);

        while self.token.is(TokenKind::Or) {
            self.advance_token()?;
            patterns.push(self.parse_match_pattern()?);
        }

        self.expect_token(TokenKind::DoubleArrow)?;

        let value = self.parse_expression()?;
        let span = self.span_from(start);

        Ok(MatchCaseType {
            id: self.generate_id(),
            pos,
            span,
            patterns,
            value,
        })
    }

    fn parse_match_pattern(&mut self) -> Result<MatchPattern, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;

        let data = if self.token.is(TokenKind::Underscore) {
            self.expect_token(TokenKind::Underscore)?;
            MatchPatternData::Underscore
        } else {
            let path = self.parse_path()?;

            let params = if self.token.is(TokenKind::LParen) {
                self.expect_token(TokenKind::LParen)?;
                let params = self.parse_list(TokenKind::Comma, TokenKind::RParen, |this| {
                    this.parse_match_pattern_param()
                })?;

                Some(params)
            } else {
                None
            };

            MatchPatternData::Ident(MatchPatternIdent { path, params })
        };

        let span = self.span_from(start);

        Ok(MatchPattern {
            id: self.generate_id(),
            pos,
            span,
            data,
        })
    }

    fn parse_match_pattern_param(&mut self) -> Result<MatchPatternParam, ParseErrorAndPos> {
        let start = self.token.span.start();
        let pos = self.token.position;

        let (mutable, name) = if self.token.is(TokenKind::Underscore) {
            self.expect_token(TokenKind::Underscore)?;

            (false, None)
        } else {
            let mutable = if self.token.is(TokenKind::Mut) {
                self.expect_token(TokenKind::Mut)?;
                true
            } else {
                false
            };

            let ident = self.expect_identifier()?;

            (mutable, Some(ident))
        };

        let span = self.span_from(start);

        Ok(MatchPatternParam {
            id: self.generate_id(),
            pos,
            span,
            mutable,
            name,
        })
    }

    fn parse_for(&mut self) -> StmtResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::For)?.position;
        let pattern = self.parse_let_pattern()?;
        self.expect_token(TokenKind::In)?;
        let expr = self.parse_expression()?;
        let block = self.parse_block_stmt()?;
        let span = self.span_from(start);

        Ok(Box::new(Stmt::create_for(
            self.generate_id(),
            pos,
            span,
            pattern,
            expr,
            block,
        )))
    }

    fn parse_while(&mut self) -> StmtResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::While)?.position;
        let expr = self.parse_expression()?;
        let block = self.parse_block_stmt()?;
        let span = self.span_from(start);

        Ok(Box::new(Stmt::create_while(
            self.generate_id(),
            pos,
            span,
            expr,
            block,
        )))
    }

    fn parse_break(&mut self) -> StmtResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Break)?.position;
        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Box::new(Stmt::create_break(self.generate_id(), pos, span)))
    }

    fn parse_continue(&mut self) -> StmtResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Continue)?.position;
        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Box::new(Stmt::create_continue(
            self.generate_id(),
            pos,
            span,
        )))
    }

    fn parse_return(&mut self) -> StmtResult {
        let start = self.token.span.start();
        let pos = self.expect_token(TokenKind::Return)?.position;
        let expr = if self.token.is(TokenKind::Semicolon) {
            None
        } else {
            let expr = self.parse_expression()?;
            Some(expr)
        };

        self.expect_semicolon()?;
        let span = self.span_from(start);

        Ok(Box::new(Stmt::create_return(
            self.generate_id(),
            pos,
            span,
            expr,
        )))
    }

    fn parse_expression(&mut self) -> ExprResult {
        let result = match self.token.kind {
            TokenKind::LBrace => self.parse_block(),
            TokenKind::If => self.parse_if(),
            TokenKind::Match => self.parse_match(),
            _ => self.parse_binary(0),
        };

        result
    }

    fn parse_binary(&mut self, precedence: u32) -> ExprResult {
        let start = self.token.span.start();
        let mut left = self.parse_unary()?;

        loop {
            let right_precedence = match self.token.kind {
                TokenKind::Eq => 1,
                TokenKind::OrOr => 2,
                TokenKind::AndAnd => 3,
                TokenKind::EqEq
                | TokenKind::NotEq
                | TokenKind::Lt
                | TokenKind::Le
                | TokenKind::Gt
                | TokenKind::Ge
                | TokenKind::EqEqEq
                | TokenKind::NeEqEq => 4,
                TokenKind::Add | TokenKind::Sub | TokenKind::Or | TokenKind::Caret => 5,
                TokenKind::Mul
                | TokenKind::Div
                | TokenKind::Modulo
                | TokenKind::And
                | TokenKind::LtLt
                | TokenKind::GtGt
                | TokenKind::GtGtGt => 6,
                TokenKind::Is | TokenKind::As => 7,
                _ => {
                    return Ok(left);
                }
            };

            if precedence >= right_precedence {
                return Ok(left);
            }

            let tok = self.advance_token()?;

            left = match tok.kind {
                TokenKind::Is | TokenKind::As => {
                    let is = tok.is(TokenKind::Is);

                    let right = Box::new(self.parse_type()?);
                    let span = self.span_from(start);
                    let expr =
                        Expr::create_conv(self.generate_id(), tok.position, span, left, right, is);

                    Box::new(expr)
                }

                _ => {
                    let right = self.parse_binary(right_precedence)?;
                    self.create_binary(tok, start, left, right)
                }
            };
        }
    }

    fn parse_unary(&mut self) -> ExprResult {
        match self.token.kind {
            TokenKind::Add | TokenKind::Sub | TokenKind::Not => {
                let start = self.token.span.start();
                let tok = self.advance_token()?;
                let op = match tok.kind {
                    TokenKind::Add => UnOp::Plus,
                    TokenKind::Sub => UnOp::Neg,
                    TokenKind::Not => UnOp::Not,
                    _ => unreachable!(),
                };

                let expr = self.parse_primary()?;
                let span = self.span_from(start);
                Ok(Box::new(Expr::create_un(
                    self.generate_id(),
                    tok.position,
                    span,
                    op,
                    expr,
                )))
            }

            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> ExprResult {
        let start = self.token.span.start();
        let mut left = self.parse_factor()?;

        loop {
            left = match self.token.kind {
                TokenKind::Dot => {
                    let tok = self.advance_token()?;
                    let rhs = self.parse_factor()?;
                    let span = self.span_from(start);

                    Box::new(Expr::create_dot(
                        self.generate_id(),
                        tok.position,
                        span,
                        left,
                        rhs,
                    ))
                }

                TokenKind::LParen => {
                    let tok = self.advance_token()?;
                    let args = self.parse_list(TokenKind::Comma, TokenKind::RParen, |p| {
                        p.parse_expression()
                    })?;
                    let span = self.span_from(start);

                    Box::new(Expr::create_call(
                        self.generate_id(),
                        tok.position,
                        span,
                        left,
                        args,
                    ))
                }

                TokenKind::LBracket => {
                    let tok = self.advance_token()?;
                    let types =
                        self.parse_list(TokenKind::Comma, TokenKind::RBracket, |p| p.parse_type())?;
                    let span = self.span_from(start);

                    Box::new(Expr::create_type_param(
                        self.generate_id(),
                        tok.position,
                        span,
                        left,
                        types,
                    ))
                }

                TokenKind::ColonColon => {
                    let tok = self.advance_token()?;
                    let rhs = self.parse_factor()?;
                    let span = self.span_from(start);

                    Box::new(Expr::create_path(
                        self.generate_id(),
                        tok.position,
                        span,
                        left,
                        rhs,
                    ))
                }

                _ => {
                    return Ok(left);
                }
            }
        }
    }

    fn create_binary(
        &mut self,
        tok: Token,
        start: u32,
        left: Box<Expr>,
        right: Box<Expr>,
    ) -> Box<Expr> {
        let op = match tok.kind {
            TokenKind::Eq => BinOp::Assign,
            TokenKind::OrOr => BinOp::Or,
            TokenKind::AndAnd => BinOp::And,
            TokenKind::EqEq => BinOp::Cmp(CmpOp::Eq),
            TokenKind::NotEq => BinOp::Cmp(CmpOp::Ne),
            TokenKind::Lt => BinOp::Cmp(CmpOp::Lt),
            TokenKind::Le => BinOp::Cmp(CmpOp::Le),
            TokenKind::Gt => BinOp::Cmp(CmpOp::Gt),
            TokenKind::Ge => BinOp::Cmp(CmpOp::Ge),
            TokenKind::EqEqEq => BinOp::Cmp(CmpOp::Is),
            TokenKind::NeEqEq => BinOp::Cmp(CmpOp::IsNot),
            TokenKind::Or => BinOp::BitOr,
            TokenKind::And => BinOp::BitAnd,
            TokenKind::Caret => BinOp::BitXor,
            TokenKind::Add => BinOp::Add,
            TokenKind::Sub => BinOp::Sub,
            TokenKind::Mul => BinOp::Mul,
            TokenKind::Div => BinOp::Div,
            TokenKind::Modulo => BinOp::Mod,
            TokenKind::LtLt => BinOp::ShiftL,
            TokenKind::GtGt => BinOp::ArithShiftR,
            TokenKind::GtGtGt => BinOp::LogicalShiftR,
            _ => panic!("unimplemented token {:?}", tok),
        };

        let span = self.span_from(start);

        Box::new(Expr::create_bin(
            self.generate_id(),
            tok.position,
            span,
            op,
            left,
            right,
        ))
    }

    fn parse_factor(&mut self) -> ExprResult {
        match self.token.kind {
            TokenKind::LParen => self.parse_parentheses(),
            TokenKind::LBrace => self.parse_block(),
            TokenKind::If => self.parse_if(),
            TokenKind::LitChar(_) => self.parse_lit_char(),
            TokenKind::LitInt(_, _, _) => self.parse_lit_int(),
            TokenKind::LitFloat(_, _) => self.parse_lit_float(),
            TokenKind::StringTail(_) | TokenKind::StringExpr(_) => self.parse_string(),
            TokenKind::Identifier(_) => self.parse_identifier(),
            TokenKind::True => self.parse_bool_literal(),
            TokenKind::False => self.parse_bool_literal(),
            TokenKind::This => self.parse_this(),
            TokenKind::Super => self.parse_super(),
            TokenKind::Or | TokenKind::OrOr => self.parse_lambda(),
            _ => Err(ParseErrorAndPos::new(
                self.token.position,
                ParseError::ExpectedFactor(self.token.name().clone()),
            )),
        }
    }

    fn parse_identifier(&mut self) -> ExprResult {
        let pos = self.token.position;
        let span = self.token.span;
        let name = self.expect_identifier()?;

        Ok(Box::new(Expr::create_ident(
            self.generate_id(),
            pos,
            span,
            name,
            None,
        )))
    }

    fn parse_parentheses(&mut self) -> ExprResult {
        let pos = self.token.position;
        let start = self.token.span.start();
        self.expect_token(TokenKind::LParen)?;

        if self.token.is(TokenKind::RParen) {
            self.advance_token()?;
            let span = self.span_from(start);
            return Ok(Box::new(Expr::create_tuple(
                self.generate_id(),
                pos,
                span,
                Vec::new(),
            )));
        }

        let expr = self.parse_expression()?;

        if self.token.kind == TokenKind::Comma {
            let mut values = vec![expr];
            let span;

            loop {
                self.expect_token(TokenKind::Comma)?;

                if self.token.kind == TokenKind::RParen {
                    self.advance_token()?;
                    span = self.span_from(start);
                    break;
                }

                let expr = self.parse_expression()?;
                values.push(expr);

                if self.token.kind == TokenKind::RParen {
                    self.advance_token()?;
                    span = self.span_from(start);
                    break;
                }
            }

            Ok(Box::new(Expr::create_tuple(
                self.generate_id(),
                pos,
                span,
                values,
            )))
        } else {
            self.expect_token(TokenKind::RParen)?;
            let span = self.span_from(start);

            Ok(Box::new(Expr::create_paren(
                self.generate_id(),
                pos,
                span,
                expr,
            )))
        }
    }

    fn parse_lit_char(&mut self) -> ExprResult {
        let span = self.token.span;
        let tok = self.advance_token()?;
        let pos = tok.position;

        if let TokenKind::LitChar(val) = tok.kind {
            Ok(Box::new(Expr::create_lit_char(
                self.generate_id(),
                pos,
                span,
                val,
            )))
        } else {
            unreachable!();
        }
    }

    fn parse_lit_int(&mut self) -> ExprResult {
        let span = self.token.span;
        let tok = self.advance_token()?;
        let pos = tok.position;

        let (value, base, suffix) = match tok.kind {
            TokenKind::LitInt(value, base, suffix) => (value, base, suffix),
            _ => unreachable!(),
        };

        let filtered = value.chars().filter(|&ch| ch != '_').collect::<String>();
        let parsed = u64::from_str_radix(&filtered, base.num());

        match parsed {
            Ok(value) => {
                let expr = Expr::create_lit_int(self.generate_id(), pos, span, value, base, suffix);
                Ok(Box::new(expr))
            }
            _ => Err(ParseErrorAndPos::new(pos, ParseError::NumberOverflow)),
        }
    }

    fn parse_lit_float(&mut self) -> ExprResult {
        let span = self.token.span;
        let tok = self.advance_token()?;
        let pos = tok.position;

        let (value, suffix) = match tok.kind {
            TokenKind::LitFloat(value, suffix) => (value, suffix),
            _ => unreachable!(),
        };

        let filtered = value.chars().filter(|&ch| ch != '_').collect::<String>();
        let parsed = filtered.parse::<f64>();

        let num = parsed.expect("unparsable float");

        let expr = Expr::create_lit_float(self.generate_id(), pos, span, num, suffix);
        Ok(Box::new(expr))
    }

    fn parse_string(&mut self) -> ExprResult {
        let span = self.token.span;
        let string = self.advance_token()?;

        match string.kind {
            TokenKind::StringTail(value) => Ok(Box::new(Expr::create_lit_str(
                self.generate_id(),
                string.position,
                span,
                value,
            ))),

            TokenKind::StringExpr(value) => {
                let start = self.token.span.start();
                let mut parts: Vec<Box<Expr>> = Vec::new();
                parts.push(Box::new(Expr::create_lit_str(
                    self.generate_id(),
                    string.position,
                    span,
                    value,
                )));

                loop {
                    let expr = self.parse_expression()?;
                    parts.push(expr);

                    if !self.token.is(TokenKind::RBrace) {
                        return Err(ParseErrorAndPos::new(
                            self.token.position,
                            ParseError::UnclosedStringTemplate,
                        ));
                    }

                    let token = self.lexer.read_string_continuation()?;
                    self.advance_token_with(token);

                    let pos = self.token.position;
                    let span = self.token.span;

                    let (value, finished) = match self.token.kind {
                        TokenKind::StringTail(ref value) => (value.clone(), true),
                        TokenKind::StringExpr(ref value) => (value.clone(), false),
                        _ => unreachable!(),
                    };

                    parts.push(Box::new(Expr::create_lit_str(
                        self.generate_id(),
                        pos,
                        span,
                        value,
                    )));

                    self.advance_token()?;

                    if finished {
                        break;
                    }
                }

                let span = self.span_from(start);

                Ok(Box::new(Expr::create_template(
                    self.generate_id(),
                    string.position,
                    span,
                    parts,
                )))
            }

            _ => unreachable!(),
        }
    }

    fn parse_bool_literal(&mut self) -> ExprResult {
        let span = self.token.span;
        let tok = self.advance_token()?;
        let value = tok.is(TokenKind::True);

        Ok(Box::new(Expr::create_lit_bool(
            self.generate_id(),
            tok.position,
            span,
            value,
        )))
    }

    fn parse_this(&mut self) -> ExprResult {
        let span = self.token.span;
        let tok = self.advance_token()?;

        Ok(Box::new(Expr::create_this(
            self.generate_id(),
            tok.position,
            span,
        )))
    }

    fn parse_super(&mut self) -> ExprResult {
        let span = self.token.span;
        let tok = self.advance_token()?;

        Ok(Box::new(Expr::create_super(
            self.generate_id(),
            tok.position,
            span,
        )))
    }

    fn parse_lambda(&mut self) -> ExprResult {
        let start = self.token.span.start();
        let tok = self.advance_token()?;
        let pos = tok.position;

        let params = if tok.kind == TokenKind::OrOr {
            // nothing to do
            Vec::new()
        } else {
            self.param_idx = 0;
            self.parse_list(TokenKind::Comma, TokenKind::Or, |p| {
                p.param_idx += 1;
                p.parse_function_param()
            })?
        };

        let return_type = if self.token.is(TokenKind::Colon) {
            self.advance_token()?;
            Some(self.parse_type()?)
        } else {
            None
        };

        let block = self.parse_block()?;

        let block = match *block {
            Expr::Block(block_type) => Some(Box::new(block_type)),
            _ => unreachable!(),
        };

        let span = self.span_from(start);

        let name = self.interner.intern("closure");

        let function = Arc::new(Function {
            id: self.generate_id(),
            kind: FunctionKind::Lambda,
            name,
            pos,
            span,
            method: self.in_class_or_module,
            is_open: false,
            is_override: false,
            is_final: false,
            is_optimize_immediately: false,
            is_pub: false,
            is_static: false,
            internal: false,
            is_abstract: false,
            is_constructor: false,
            is_test: false,
            params,
            return_type,
            block,
            type_params: None,
        });

        Ok(Box::new(Expr::create_lambda(function)))
    }

    fn expect_identifier(&mut self) -> Result<Name, ParseErrorAndPos> {
        let tok = self.advance_token()?;

        if let TokenKind::Identifier(ref value) = tok.kind {
            let interned = self.interner.intern(value);

            Ok(interned)
        } else {
            Err(ParseErrorAndPos::new(
                tok.position,
                ParseError::ExpectedIdentifier(tok.name()),
            ))
        }
    }

    fn expect_semicolon(&mut self) -> Result<Token, ParseErrorAndPos> {
        self.expect_token(TokenKind::Semicolon)
    }

    fn expect_token(&mut self, kind: TokenKind) -> Result<Token, ParseErrorAndPos> {
        if self.token.kind == kind {
            let token = self.advance_token()?;

            Ok(token)
        } else {
            Err(ParseErrorAndPos::new(
                self.token.position,
                ParseError::ExpectedToken(kind.name().into(), self.token.name()),
            ))
        }
    }

    fn advance_token(&mut self) -> Result<Token, ParseErrorAndPos> {
        let token = self.lexer.read_token()?;
        Ok(self.advance_token_with(token))
    }

    fn advance_token_with(&mut self, token: Token) -> Token {
        self.last_end = if self.token.span.is_valid() {
            Some(self.token.span.end())
        } else {
            None
        };

        mem::replace(&mut self.token, token)
    }

    fn span_from(&self, start: u32) -> Span {
        Span::new(start, self.last_end.unwrap() - start)
    }

    fn generate_global_initializer(&mut self, global: &Global, initializer: Box<Expr>) -> Function {
        let builder = Builder::new();
        let mut block = builder.build_block();

        let var = builder.build_ident(self.generate_id(), global.name);
        let assignment = builder.build_initializer_assign(self.generate_id(), var, initializer);

        block.add_expr(self.generate_id(), assignment);

        let mut fct = builder.build_fct(global.name);
        fct.block(block.build(self.generate_id()));
        fct.build(self.generate_id())
    }

    fn generate_constructor(
        &mut self,
        cls: &mut Class,
        ctor_params: Vec<ConstructorParam>,
    ) -> Function {
        let builder = Builder::new();
        let mut block = builder.build_block();

        if let Some(ref parent_class) = cls.parent_class {
            let expr = Expr::create_delegation(
                self.generate_id(),
                parent_class.pos,
                parent_class.span,
                parent_class.params.clone(),
            );

            block.add_expr(self.generate_id(), Box::new(expr));
        }

        for param in ctor_params.iter().filter(|param| param.field) {
            let this = builder.build_this(self.generate_id());
            let lhs = builder.build_dot(
                self.generate_id(),
                this,
                builder.build_ident(self.generate_id(), param.name),
            );
            let rhs = builder.build_ident(self.generate_id(), param.name);
            let ass = builder.build_initializer_assign(self.generate_id(), lhs, rhs);

            block.add_expr(self.generate_id(), ass);
        }

        for field in cls.fields.iter().filter(|field| field.expr.is_some()) {
            let this = builder.build_this(self.generate_id());
            let lhs = builder.build_dot(
                self.generate_id(),
                this,
                builder.build_ident(self.generate_id(), field.name),
            );
            let ass = builder.build_initializer_assign(
                self.generate_id(),
                lhs,
                field.expr.as_ref().unwrap().clone(),
            );

            block.add_expr(self.generate_id(), ass);
        }

        block.add_stmts(mem::replace(&mut cls.initializers, Vec::new()));

        let mut fct = builder.build_fct(cls.name);

        for param in &ctor_params {
            fct.add_param(
                self.generate_id(),
                param.pos,
                param.name,
                param.data_type.clone(),
                param.variadic,
            );
        }

        fct.is_method(true)
            .is_public(true)
            .constructor(true)
            .block(block.build(self.generate_id()));

        fct.build(self.generate_id())
    }
}

#[derive(Debug)]
pub struct NodeIdGenerator {
    value: RefCell<usize>,
}

impl NodeIdGenerator {
    pub fn new() -> NodeIdGenerator {
        NodeIdGenerator {
            value: RefCell::new(1),
        }
    }

    pub fn next(&self) -> NodeId {
        let value = *self.value.borrow();
        *self.value.borrow_mut() += 1;

        NodeId(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::*;
    use crate::interner::*;

    use crate::error::ParseError;
    use crate::lexer::position::Position;
    use crate::parser::Parser;

    fn parse_expr(code: &'static str) -> (Box<Expr>, Interner) {
        let mut interner = Interner::new();

        let expr = {
            let mut parser = Parser::from_string(code, &mut interner);
            assert!(parser.init().is_ok());

            let result = parser.parse_expression();

            if let Err(ref msg) = result {
                println!("error parsing: {:?}", msg);
            }

            result.unwrap()
        };

        (expr, interner)
    }

    fn err_expr(code: &'static str, msg: ParseError, line: u32, col: u32) {
        let err = {
            let mut interner = Interner::new();
            let mut parser = Parser::from_string(code, &mut interner);

            assert!(parser.init().is_ok());
            parser.parse_expression().unwrap_err()
        };

        assert_eq!(msg, err.error);
        assert_eq!(line, err.pos.line);
        assert_eq!(col, err.pos.column);
    }

    fn parse_stmt(code: &'static str) -> Box<Stmt> {
        let mut interner = Interner::new();
        let mut parser = Parser::from_string(code, &mut interner);
        assert!(parser.init().is_ok());

        parser.parse_statement().unwrap()
    }

    fn err_stmt(code: &'static str, msg: ParseError, line: u32, col: u32) {
        let err = {
            let mut interner = Interner::new();
            let mut parser = Parser::from_string(code, &mut interner);

            assert!(parser.init().is_ok());
            parser.parse_statement().unwrap_err()
        };

        assert_eq!(msg, err.error);
        assert_eq!(line, err.pos.line);
        assert_eq!(col, err.pos.column);
    }

    fn parse_type(code: &'static str) -> (Type, Interner) {
        let mut interner = Interner::new();
        let ty = {
            let mut parser = Parser::from_string(code, &mut interner);
            assert!(parser.init().is_ok());

            parser.parse_type().unwrap()
        };

        (ty, interner)
    }

    fn parse(code: &'static str) -> (File, Interner) {
        let mut interner = Interner::new();

        let file = Parser::from_string(code, &mut interner).parse().unwrap();

        (file, interner)
    }

    fn parse_err(code: &'static str, msg: ParseError, line: u32, col: u32) {
        let mut interner = Interner::new();

        let err = Parser::from_string(code, &mut interner)
            .parse()
            .unwrap_err();

        assert_eq!(msg, err.error);
        assert_eq!(line, err.pos.line);
        assert_eq!(col, err.pos.column);
    }

    #[test]
    fn parse_ident() {
        let (expr, interner) = parse_expr("a");

        let ident = expr.to_ident().unwrap();
        assert_eq!("a", *interner.str(ident.name));
    }

    #[test]
    fn parse_number() {
        let (expr, _) = parse_expr("10");

        let lit = expr.to_lit_int().unwrap();
        assert_eq!(10, lit.value);
    }

    #[test]
    fn parse_number_with_underscore() {
        let (expr, _) = parse_expr("1____0");

        let lit = expr.to_lit_int().unwrap();
        assert_eq!(10, lit.value);
    }

    #[test]
    fn parse_string() {
        let (expr, _) = parse_expr("\"abc\"");

        let lit = expr.to_lit_str().unwrap();
        assert_eq!("abc", &lit.value);
    }

    #[test]
    fn parse_true() {
        let (expr, _) = parse_expr("true");

        let lit = expr.to_lit_bool().unwrap();
        assert_eq!(true, lit.value);
    }

    #[test]
    fn parse_false() {
        let (expr, _) = parse_expr("true");

        let lit = expr.to_lit_bool().unwrap();
        assert_eq!(true, lit.value);
    }

    #[test]
    fn parse_field_access() {
        let (expr, interner) = parse_expr("obj.field");
        let dot = expr.to_dot().unwrap();

        let ident = dot.lhs.to_ident().unwrap();
        assert_eq!("obj", *interner.str(ident.name));

        let ident = dot.rhs.to_ident().unwrap();
        assert_eq!("field", *interner.str(ident.name));
    }

    #[test]
    fn parse_field_negated() {
        let (expr, _) = parse_expr("-obj.field");
        assert!(expr.to_un().unwrap().opnd.is_dot());
    }

    #[test]
    fn parse_field_non_ident() {
        let (expr, interner) = parse_expr("bar.12");
        let dot = expr.to_dot().unwrap();

        let ident = dot.lhs.to_ident().unwrap();
        assert_eq!("bar", *interner.str(ident.name));

        assert_eq!(12, dot.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_self() {
        let (expr, _) = parse_expr("self");

        assert!(expr.is_this());
    }

    #[test]
    fn parse_neg() {
        let (expr, _) = parse_expr("-1");

        let un = expr.to_un().unwrap();
        assert_eq!(UnOp::Neg, un.op);

        assert!(un.opnd.is_lit_int());
    }

    #[test]
    fn parse_neg_twice() {
        let (expr, _) = parse_expr("-(-3)");

        let neg1 = expr.to_un().unwrap();
        assert_eq!(UnOp::Neg, neg1.op);

        let neg2 = neg1.opnd.to_paren().unwrap().expr.to_un().unwrap();
        assert_eq!(UnOp::Neg, neg2.op);

        assert!(neg2.opnd.is_lit_int());
    }

    #[test]
    fn parse_neg_twice_without_parentheses() {
        err_expr("- -2", ParseError::ExpectedFactor("-".into()), 1, 3);
    }

    #[test]
    fn parse_unary_plus() {
        let (expr, _) = parse_expr("+2");

        let add = expr.to_un().unwrap();
        assert_eq!(UnOp::Plus, add.op);

        assert!(add.opnd.is_lit_int());
    }

    #[test]
    fn parse_unary_plus_twice_without_parentheses() {
        err_expr("+ +4", ParseError::ExpectedFactor("+".into()), 1, 3);
    }

    #[test]
    fn parse_unary_plus_twice() {
        let (expr, _) = parse_expr("+(+9)");

        let add1 = expr.to_un().unwrap();
        assert_eq!(UnOp::Plus, add1.op);

        let add2 = add1.opnd.to_paren().unwrap().expr.to_un().unwrap();
        assert_eq!(UnOp::Plus, add2.op);
        assert!(add2.opnd.is_lit_int());
    }

    #[test]
    fn parse_mul() {
        let (expr, _) = parse_expr("6*3");

        let mul = expr.to_bin().unwrap();
        assert_eq!(BinOp::Mul, mul.op);
        assert_eq!(6, mul.lhs.to_lit_int().unwrap().value);
        assert_eq!(3, mul.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_multiple_muls() {
        let (expr, _) = parse_expr("6*3*4");

        let mul1 = expr.to_bin().unwrap();
        assert_eq!(BinOp::Mul, mul1.op);

        let mul2 = mul1.lhs.to_bin().unwrap();
        assert_eq!(BinOp::Mul, mul2.op);
        assert_eq!(6, mul2.lhs.to_lit_int().unwrap().value);
        assert_eq!(3, mul2.rhs.to_lit_int().unwrap().value);

        assert_eq!(4, mul1.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_div() {
        let (expr, _) = parse_expr("4/5");

        let div = expr.to_bin().unwrap();
        assert_eq!(BinOp::Div, div.op);
        assert_eq!(4, div.lhs.to_lit_int().unwrap().value);
        assert_eq!(5, div.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_mod() {
        let (expr, _) = parse_expr("2%15");

        let div = expr.to_bin().unwrap();
        assert_eq!(BinOp::Mod, div.op);
        assert_eq!(2, div.lhs.to_lit_int().unwrap().value);
        assert_eq!(15, div.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_add() {
        let (expr, _) = parse_expr("2+3");

        let add = expr.to_bin().unwrap();
        assert_eq!(BinOp::Add, add.op);
        assert_eq!(2, add.lhs.to_lit_int().unwrap().value);
        assert_eq!(3, add.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_add_left_associativity() {
        let (expr, _) = parse_expr("1+2+3");

        let add = expr.to_bin().unwrap();
        assert_eq!(3, add.rhs.to_lit_int().unwrap().value);

        let lhs = add.lhs.to_bin().unwrap();
        assert_eq!(1, lhs.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, lhs.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_add_right_associativity_via_parens() {
        let (expr, _) = parse_expr("1+(2+3)");

        let add = expr.to_bin().unwrap();
        assert_eq!(1, add.lhs.to_lit_int().unwrap().value);

        let rhs = add.rhs.to_paren().unwrap().expr.to_bin().unwrap();
        assert_eq!(2, rhs.lhs.to_lit_int().unwrap().value);
        assert_eq!(3, rhs.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_sub() {
        let (expr, _) = parse_expr("1-2");

        let add = expr.to_bin().unwrap();
        assert_eq!(BinOp::Sub, add.op);
        assert_eq!(1, add.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, add.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_or() {
        let (expr, _) = parse_expr("1||2");

        let add = expr.to_bin().unwrap();
        assert_eq!(BinOp::Or, add.op);
        assert_eq!(1, add.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, add.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_and() {
        let (expr, _) = parse_expr("1&&2");

        let add = expr.to_bin().unwrap();
        assert_eq!(BinOp::And, add.op);
        assert_eq!(1, add.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, add.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_bit_or() {
        let (expr, _) = parse_expr("1|2");

        let or = expr.to_bin().unwrap();
        assert_eq!(BinOp::BitOr, or.op);
        assert_eq!(1, or.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, or.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_bit_and() {
        let (expr, _) = parse_expr("1&2");

        let and = expr.to_bin().unwrap();
        assert_eq!(BinOp::BitAnd, and.op);
        assert_eq!(1, and.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, and.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_bit_xor() {
        let (expr, _) = parse_expr("1^2");

        let xor = expr.to_bin().unwrap();
        assert_eq!(BinOp::BitXor, xor.op);
        assert_eq!(1, xor.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, xor.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_lt() {
        let (expr, _) = parse_expr("1<2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Lt), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_le() {
        let (expr, _) = parse_expr("1<=2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Le), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_gt() {
        let (expr, _) = parse_expr("1>2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Gt), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_ge() {
        let (expr, _) = parse_expr("1>=2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Ge), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_eq() {
        let (expr, _) = parse_expr("1==2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Eq), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_ne() {
        let (expr, _) = parse_expr("1!=2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Ne), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_is_not() {
        let (expr, _) = parse_expr("1!==2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::IsNot), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_is() {
        let (expr, _) = parse_expr("1===2");

        let cmp = expr.to_bin().unwrap();
        assert_eq!(BinOp::Cmp(CmpOp::Is), cmp.op);
        assert_eq!(1, cmp.lhs.to_lit_int().unwrap().value);
        assert_eq!(2, cmp.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_assign() {
        let (expr, _) = parse_expr("a=4");

        let assign = expr.to_bin().unwrap();
        assert!(assign.lhs.is_ident());
        assert_eq!(BinOp::Assign, assign.op);
        assert_eq!(4, assign.rhs.to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_shift_right() {
        let (expr, _) = parse_expr("a>>4");

        let bin = expr.to_bin().unwrap();
        assert_eq!(BinOp::ArithShiftR, bin.op);
    }

    #[test]
    fn parse_unsigned_shift_right() {
        let (expr, _) = parse_expr("a>>>4");

        let bin = expr.to_bin().unwrap();
        assert_eq!(BinOp::LogicalShiftR, bin.op);
    }

    #[test]
    fn parse_left() {
        let (expr, _) = parse_expr("a<<4");

        let bin = expr.to_bin().unwrap();
        assert_eq!(BinOp::ShiftL, bin.op);
    }

    #[test]
    fn parse_call_without_params() {
        let (expr, interner) = parse_expr("fname()");

        let call = expr.to_call().unwrap();
        assert_eq!("fname", *interner.str(call.callee.to_ident().unwrap().name));
        assert_eq!(0, call.args.len());
    }

    #[test]
    fn parse_call_with_params() {
        let (expr, interner) = parse_expr("fname2(1,2,3)");

        let call = expr.to_call().unwrap();
        assert_eq!(
            "fname2",
            *interner.str(call.callee.to_ident().unwrap().name)
        );
        assert_eq!(3, call.args.len());

        for i in 0..3 {
            let lit = call.args[i as usize].to_lit_int().unwrap();
            assert_eq!(i + 1, lit.value);
        }
    }

    #[test]
    fn parse_function() {
        let (prog, interner) = parse("fn b() { }");
        let fct = prog.fct0();

        assert_eq!("b", *interner.str(fct.name));
        assert_eq!(0, fct.params.len());
        assert!(fct.return_type.is_none());
        assert_eq!(Position::new(1, 1), fct.pos);
    }

    #[test]
    fn parse_function_with_single_param() {
        let (p1, interner1) = parse("fn f(a:int) { }");
        let f1 = p1.fct0();

        let (p2, interner2) = parse("fn f(a:int,) { }");
        let f2 = p2.fct0();

        assert_eq!(f1.params.len(), 1);
        assert_eq!(f2.params.len(), 1);

        let p1 = &f1.params[0];
        let p2 = &f2.params[0];

        assert_eq!("a", *interner1.str(p1.name));
        assert_eq!("a", *interner2.str(p2.name));

        assert_eq!(
            "int",
            *interner1.str(p1.data_type.to_basic().unwrap().name())
        );
        assert_eq!(
            "int",
            *interner2.str(p2.data_type.to_basic().unwrap().name())
        );
    }

    #[test]
    fn parse_function_with_multiple_params() {
        let (p1, interner1) = parse("fn f(a:int, b:str) { }");
        let f1 = p1.fct0();

        let (p2, interner2) = parse("fn f(a:int, b:str,) { }");
        let f2 = p2.fct0();

        let p1a = &f1.params[0];
        let p1b = &f1.params[1];
        let p2a = &f2.params[0];
        let p2b = &f2.params[1];

        assert_eq!("a", *interner1.str(p1a.name));
        assert_eq!("a", *interner2.str(p2a.name));

        assert_eq!("b", *interner1.str(p1b.name));
        assert_eq!("b", *interner2.str(p2b.name));

        assert_eq!(
            "int",
            *interner1.str(p1a.data_type.to_basic().unwrap().name())
        );
        assert_eq!(
            "int",
            *interner2.str(p2a.data_type.to_basic().unwrap().name())
        );

        assert_eq!(
            "str",
            *interner1.str(p1b.data_type.to_basic().unwrap().name())
        );
        assert_eq!(
            "str",
            *interner2.str(p2b.data_type.to_basic().unwrap().name())
        );
    }

    #[test]
    fn parse_let_without_type() {
        let stmt = parse_stmt("let a = 1;");
        let var = stmt.to_let().unwrap();

        assert_eq!(false, var.mutable);
        assert!(var.data_type.is_none());
        assert!(var.expr.as_ref().unwrap().is_lit_int());
    }

    #[test]
    fn parse_var_without_type() {
        let stmt = parse_stmt("var a = 1;");
        let var = stmt.to_let().unwrap();

        assert_eq!(true, var.mutable);
        assert!(var.data_type.is_none());
        assert!(var.expr.as_ref().unwrap().is_lit_int());
    }

    #[test]
    fn parse_let_with_type() {
        let stmt = parse_stmt("let x : int = 1;");
        let var = stmt.to_let().unwrap();

        assert_eq!(false, var.mutable);
        assert!(var.data_type.is_some());
        assert!(var.expr.as_ref().unwrap().is_lit_int());
    }

    #[test]
    fn parse_let_underscore() {
        let stmt = parse_stmt("let _ = 1;");
        let let_decl = stmt.to_let().unwrap();

        assert!(let_decl.pattern.is_underscore());
    }

    #[test]
    fn parse_let_tuple() {
        let stmt = parse_stmt("let (mut a, b, (c, d)) = 1;");
        let let_decl = stmt.to_let().unwrap();

        assert!(let_decl.pattern.is_tuple());
        let tuple = let_decl.pattern.to_tuple().unwrap();
        let first = tuple.parts.first().unwrap();
        assert!(first.is_ident());
        assert!(first.to_ident().unwrap().mutable);
        assert!(tuple.parts.last().unwrap().is_tuple());
    }

    #[test]
    fn parse_let_ident() {
        let stmt = parse_stmt("let x = 1;");
        let let_decl = stmt.to_let().unwrap();

        assert!(let_decl.pattern.is_ident());
    }

    #[test]
    fn parse_let_ident_mut() {
        let stmt = parse_stmt("let mut x = 1;");
        let let_decl = stmt.to_let().unwrap();

        assert!(let_decl.pattern.is_ident());
        assert!(let_decl.pattern.to_ident().unwrap().mutable);
    }

    #[test]
    fn parse_var_with_type() {
        let stmt = parse_stmt("var x : int = 1;");
        let var = stmt.to_let().unwrap();

        assert_eq!(true, var.mutable);
        assert!(var.data_type.is_some());
        assert!(var.expr.as_ref().unwrap().is_lit_int());
    }

    #[test]
    fn parse_let_with_type_but_without_assignment() {
        let stmt = parse_stmt("let x : int;");
        let var = stmt.to_let().unwrap();

        assert_eq!(false, var.mutable);
        assert!(var.data_type.is_some());
        assert!(var.expr.is_none());
    }

    #[test]
    fn parse_var_with_type_but_without_assignment() {
        let stmt = parse_stmt("var x : int;");
        let var = stmt.to_let().unwrap();

        assert_eq!(true, var.mutable);
        assert!(var.data_type.is_some());
        assert!(var.expr.is_none());
    }

    #[test]
    fn parse_let_without_type_and_assignment() {
        let stmt = parse_stmt("let x;");
        let var = stmt.to_let().unwrap();

        assert_eq!(false, var.mutable);
        assert!(var.data_type.is_none());
        assert!(var.expr.is_none());
    }

    #[test]
    fn parse_var_without_type_and_assignment() {
        let stmt = parse_stmt("var x;");
        let var = stmt.to_let().unwrap();

        assert_eq!(true, var.mutable);
        assert!(var.data_type.is_none());
        assert!(var.expr.is_none());
    }

    #[test]
    fn parse_multiple_functions() {
        let (prog, interner) = parse("fn f() { } fn g() { }");

        let f = prog.fct0();
        assert_eq!("f", *interner.str(f.name));
        assert_eq!(false, f.method);
        assert_eq!(Position::new(1, 1), f.pos);

        let g = prog.fct(1);
        assert_eq!("g", *interner.str(g.name));
        assert_eq!(false, g.method);
        assert_eq!(Position::new(1, 12), g.pos);
    }

    #[test]
    fn parse_expr_stmt() {
        let stmt = parse_stmt("1;");
        let expr = stmt.to_expr().unwrap();

        assert!(expr.expr.is_lit_int());
    }

    #[test]
    fn parse_expr_stmt_without_semicolon() {
        err_stmt(
            "1",
            ParseError::ExpectedToken(";".into(), "<<EOF>>".into()),
            1,
            2,
        );
    }

    #[test]
    fn parse_if() {
        let (expr, _) = parse_expr("if true { 2; } else { 3; }");
        let ifexpr = expr.to_if().unwrap();

        assert!(ifexpr.cond.is_lit_bool());
        assert!(ifexpr.else_block.is_some());
    }

    #[test]
    fn parse_if_without_else() {
        let (expr, _) = parse_expr("if true { 2; }");
        let ifexpr = expr.to_if().unwrap();

        assert!(ifexpr.cond.is_lit_bool());
        assert!(ifexpr.else_block.is_none());
    }

    #[test]
    fn parse_while() {
        let stmt = parse_stmt("while true { 2; }");
        let whilestmt = stmt.to_while().unwrap();

        assert!(whilestmt.cond.is_lit_bool());
        assert!(whilestmt.block.is_expr());
    }

    #[test]
    fn parse_empty_block() {
        let (expr, _) = parse_expr("{}");
        let block = expr.to_block().unwrap();

        assert_eq!(0, block.stmts.len());
    }

    #[test]
    fn parse_block_with_one_stmt() {
        let (expr, _) = parse_expr("{ 1; 2 }");
        let block = expr.to_block().unwrap();

        assert_eq!(1, block.stmts.len());

        let expr = &block.stmts[0].to_expr().unwrap().expr;
        assert_eq!(1, expr.to_lit_int().unwrap().value);

        assert_eq!(2, block.expr.as_ref().unwrap().to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_block_with_multiple_stmts() {
        let (expr, _) = parse_expr("{ 1; 2; }");
        let block = expr.to_block().unwrap();

        assert_eq!(2, block.stmts.len());

        let expr = &block.stmts[0].to_expr().unwrap().expr;
        assert_eq!(1, expr.to_lit_int().unwrap().value);

        let expr = &block.stmts[1].to_expr().unwrap().expr;
        assert_eq!(2, expr.to_lit_int().unwrap().value);

        assert!(block.expr.is_none());
    }

    #[test]
    fn parse_break() {
        let stmt = parse_stmt("break;");

        assert!(stmt.is_break());
    }

    #[test]
    fn parse_continue() {
        let stmt = parse_stmt("continue;");

        assert!(stmt.is_continue());
    }

    #[test]
    fn parse_return_value() {
        let stmt = parse_stmt("return 1;");
        let ret = stmt.to_return().unwrap();

        assert_eq!(1, ret.expr.as_ref().unwrap().to_lit_int().unwrap().value);
    }

    #[test]
    fn parse_return() {
        let stmt = parse_stmt("return;");
        let ret = stmt.to_return().unwrap();

        assert!(ret.expr.is_none());
    }

    #[test]
    fn parse_else() {
        err_stmt("else", ParseError::MisplacedElse, 1, 1);
    }

    #[test]
    fn parse_type_basic() {
        let (ty, interner) = parse_type("bla");
        let basic = ty.to_basic().unwrap();

        assert_eq!(0, basic.params.len());
        assert_eq!("bla", *interner.str(basic.name()));
    }

    #[test]
    fn parse_type_basic_mod() {
        let (ty, interner) = parse_type("foo::bla");
        let basic = ty.to_basic().unwrap();

        assert_eq!(0, basic.params.len());
        assert_eq!(2, basic.path.names.len());
        assert_eq!("foo", *interner.str(basic.path.names[0]));
        assert_eq!("bla", *interner.str(basic.path.names[1]));
    }

    #[test]
    fn parse_type_basic_with_params() {
        let (ty, interner) = parse_type("Foo[A, B]");
        let basic = ty.to_basic().unwrap();

        assert_eq!(2, basic.params.len());
        assert_eq!("Foo", *interner.str(basic.name()));
        assert_eq!(
            "A",
            *interner.str(basic.params[0].to_basic().unwrap().name())
        );
        assert_eq!(
            "B",
            *interner.str(basic.params[1].to_basic().unwrap().name())
        );
    }

    #[test]
    fn parse_type_lambda_no_params() {
        let (ty, _) = parse_type("(): ()");
        let fct = ty.to_fct().unwrap();

        assert_eq!(0, fct.params.len());
        assert!(fct.ret.is_unit());
    }

    #[test]
    fn parse_type_lambda_one_param() {
        let (ty, interner) = parse_type("(A): B");
        let fct = ty.to_fct().unwrap();

        assert_eq!(1, fct.params.len());
        assert_eq!("A", *interner.str(fct.params[0].to_basic().unwrap().name()));
        assert_eq!("B", *interner.str(fct.ret.to_basic().unwrap().name()));
    }

    #[test]
    fn parse_type_lambda_two_params() {
        let (ty, interner) = parse_type("(A, B): C");
        let fct = ty.to_fct().unwrap();

        assert_eq!(2, fct.params.len());
        assert_eq!("A", *interner.str(fct.params[0].to_basic().unwrap().name()));
        assert_eq!("B", *interner.str(fct.params[1].to_basic().unwrap().name()));
        assert_eq!("C", *interner.str(fct.ret.to_basic().unwrap().name()));
    }

    #[test]
    fn parse_type_unit() {
        let (ty, _) = parse_type("()");
        let ty = ty.to_tuple().unwrap();

        assert!(ty.subtypes.is_empty());
    }

    #[test]
    fn parse_type_tuple_with_one_type() {
        let (ty, interner) = parse_type("(c)");

        let subtypes = &ty.to_tuple().unwrap().subtypes;
        assert_eq!(1, subtypes.len());

        let ty = subtypes[0].to_basic().unwrap();
        assert_eq!("c", *interner.str(ty.name()));
    }

    #[test]
    fn parse_type_tuple_with_two_types() {
        let (ty, interner) = parse_type("(a, b)");

        let subtypes = &ty.to_tuple().unwrap().subtypes;
        assert_eq!(2, subtypes.len());

        let ty1 = subtypes[0].to_basic().unwrap();
        assert_eq!("a", *interner.str(ty1.name()));

        let ty2 = subtypes[1].to_basic().unwrap();
        assert_eq!("b", *interner.str(ty2.name()));
    }

    #[test]
    fn parse_method() {
        let (prog, interner) = parse(
            "class Foo {
            fn zero(): int { return 0; }
            fn id(a: String): String { return a; }
        }",
        );

        let cls = prog.cls0();
        assert_eq!(0, cls.fields.len());
        assert_eq!(2, cls.methods.len());

        let mtd1 = &cls.methods[0];
        assert_eq!("zero", *interner.str(mtd1.name));
        assert_eq!(0, mtd1.params.len());
        assert_eq!(true, mtd1.method);
        let rt1 = mtd1
            .return_type
            .as_ref()
            .unwrap()
            .to_basic()
            .unwrap()
            .name();
        assert_eq!("int", *interner.str(rt1));

        let mtd2 = &cls.methods[1];
        assert_eq!("id", *interner.str(mtd2.name));
        assert_eq!(1, mtd2.params.len());
        assert_eq!(true, mtd2.method);
        let rt2 = mtd2
            .return_type
            .as_ref()
            .unwrap()
            .to_basic()
            .unwrap()
            .name();
        assert_eq!("String", *interner.str(rt2));
    }

    #[test]
    fn parse_abstract_method() {
        let (prog, _) = parse(
            "class Foo {
            @abstract fn zero();
            fn foo();
        }",
        );

        let cls = prog.cls0();
        assert_eq!(0, cls.fields.len());
        assert_eq!(2, cls.methods.len());

        let mtd1 = &cls.methods[0];
        assert_eq!(true, mtd1.is_abstract);

        let mtd2 = &cls.methods[1];
        assert_eq!(false, mtd2.is_abstract);
    }

    #[test]
    fn parse_class() {
        let (prog, interner) = parse("class Foo");
        let class = prog.cls0();

        assert_eq!(0, class.fields.len());
        assert_eq!(false, class.is_open);
        assert_eq!(false, class.is_abstract);
        assert_eq!(Position::new(1, 1), class.pos);
        assert_eq!("Foo", *interner.str(class.name));
    }

    #[test]
    fn parse_class_abstract() {
        let (prog, interner) = parse("@abstract class Foo");
        let class = prog.cls0();

        assert_eq!(true, class.is_abstract);
        assert_eq!("Foo", *interner.str(class.name));
    }

    #[test]
    fn parse_class_with_parens_but_no_params() {
        let (prog, interner) = parse("@open class Foo()");
        let class = prog.cls0();

        assert_eq!(0, class.fields.len());
        assert_eq!(true, class.is_open);
        assert_eq!(Position::new(1, 7), class.pos);
        assert_eq!("Foo", *interner.str(class.name));
    }

    #[test]
    fn parse_class_with_param() {
        let (prog, _) = parse("class Foo(a: int)");
        let class = prog.cls0();
        let ctor = class.constructor.clone().unwrap();

        assert_eq!(0, class.fields.len());
        assert_eq!(true, class.has_constructor);
        assert_eq!(1, ctor.params.len());
    }

    #[test]
    fn parse_class_with_param_var() {
        let (prog, _) = parse("class Foo(var a: int)");
        let class = prog.cls0();

        assert_eq!(1, class.fields.len());
        assert_eq!(true, class.fields[0].mutable);
        assert_eq!(true, class.has_constructor);
        assert_eq!(1, class.constructor.clone().unwrap().params.len());
    }

    #[test]
    fn parse_class_with_param_let() {
        let (prog, _) = parse("class Foo(let a: int)");
        let class = prog.cls0();
        let ctor = class.constructor.clone().unwrap();

        assert_eq!(1, class.fields.len());
        assert_eq!(false, class.fields[0].mutable);
        assert_eq!(true, class.has_constructor);
        assert_eq!(1, ctor.params.len());
    }

    #[test]
    fn parse_class_with_params() {
        let (prog, _) = parse("class Foo(a: int, b: int)");
        let class = prog.cls0();

        assert_eq!(0, class.fields.len());
        assert_eq!(2, class.constructor.clone().unwrap().params.len());
    }

    #[test]
    fn parse_class_with_parent_class() {
        let (prog, _interner) = parse("class Foo: Bar");
        let class = prog.cls0();

        assert!(class.parent_class.is_some());
    }

    #[test]
    fn parse_class_with_open() {
        let (prog, _) = parse("@open class Foo");
        let class = prog.cls0();

        assert_eq!(true, class.is_open);
    }

    #[test]
    fn parse_class2() {
        let (prog, _) = parse("class2 Foo { a: Int64, b: Bool }");
        let class = prog.cls0();
        assert_eq!(class.fields.len(), 2);

        let (prog, _) = parse("class2 Foo(a: Int64, b: Bool)");
        let class = prog.cls0();
        assert_eq!(class.fields.len(), 2);

        let (prog, _) = parse("class2 Foo");
        let class = prog.cls0();
        assert!(class.fields.is_empty());
    }

    #[test]
    fn parse_method_invocation() {
        let (expr, _) = parse_expr("a.foo()");
        let call = expr.to_call().unwrap();
        assert!(call.callee.is_dot());
        assert_eq!(0, call.args.len());

        let (expr, _) = parse_expr("a.foo(1)");
        let call = expr.to_call().unwrap();
        assert!(call.callee.is_dot());
        assert_eq!(1, call.args.len());

        let (expr, _) = parse_expr("a.foo(1,2)");
        let call = expr.to_call().unwrap();
        assert!(call.callee.is_dot());
        assert_eq!(2, call.args.len());
    }

    #[test]
    fn parse_array_index() {
        let (expr, interner) = parse_expr("a(b)");
        let call = expr.to_call().unwrap();
        assert_eq!("a", *interner.str(call.callee.to_ident().unwrap().name));
        assert_eq!(1, call.args.len());
        assert_eq!("b", *interner.str(call.args[0].to_ident().unwrap().name));
    }

    #[test]
    fn parse_field() {
        let (prog, interner) = parse("class A { var f1: int; let f2: int = 0; }");
        let cls = prog.cls0();

        let f1 = &cls.fields[0];
        assert_eq!("f1", &interner.str(f1.name).to_string());
        assert_eq!(true, f1.mutable);

        let f2 = &cls.fields[1];
        assert_eq!("f2", &interner.str(f2.name).to_string());
        assert_eq!(false, f2.mutable);
    }

    #[test]
    fn parse_open_method() {
        let (prog, _) = parse("class A { @open fn f() {} fn g() {} }");
        let cls = prog.cls0();

        let m1 = &cls.methods[0];
        assert_eq!(true, m1.is_open);

        let m2 = &cls.methods[1];
        assert_eq!(false, m2.is_open);
    }

    #[test]
    fn parse_override_method() {
        let (prog, _) = parse(
            "class A { fn f() {}
                @override fn g() {}
                @open fn h() {} }",
        );
        let cls = prog.cls0();

        let m1 = &cls.methods[0];
        assert_eq!(false, m1.is_override);
        assert_eq!(false, m1.is_open);

        let m2 = &cls.methods[1];
        assert_eq!(true, m2.is_override);
        assert_eq!(false, m2.is_open);

        let m3 = &cls.methods[2];
        assert_eq!(false, m3.is_override);
        assert_eq!(true, m3.is_open);
    }

    #[test]
    fn parse_parent_class_params() {
        let (prog, _) = parse("class A: B(1, 2)");
        let cls = prog.cls0();

        let parent_class = cls.parent_class.as_ref().unwrap();
        assert_eq!(2, parent_class.params.len());
    }

    #[test]
    fn parse_final_method() {
        let (prog, _) = parse("@open class A { @final @override fn g() {} }");
        let cls = prog.cls0();

        let m1 = &cls.methods[0];
        assert_eq!(true, m1.is_override);
        assert_eq!(false, m1.is_open);
        assert_eq!(true, m1.is_final);
    }

    #[test]
    fn parse_is_expr() {
        let (expr, _) = parse_expr("a is String");
        let expr = expr.to_conv().unwrap();
        assert_eq!(true, expr.object.is_ident());
        assert_eq!(true, expr.is);
    }

    #[test]
    fn parse_as_expr() {
        let (expr, _) = parse_expr("a as String");
        let expr = expr.to_conv().unwrap();
        assert_eq!(true, expr.object.is_ident());
        assert_eq!(false, expr.is);
    }

    #[test]
    fn parse_internal() {
        let (prog, _) = parse("@internal fn foo();");
        let fct = prog.fct0();
        assert!(fct.internal);
    }

    #[test]
    fn parse_function_without_body() {
        let (prog, _) = parse("fn foo();");
        let fct = prog.fct0();
        assert!(fct.block.is_none());
    }

    #[test]
    fn parse_internal_class() {
        let (prog, _) = parse("@internal class Foo {}");
        let cls = prog.cls0();
        assert!(cls.internal);
    }

    #[test]
    fn parse_struct_empty() {
        let (prog, interner) = parse("struct Foo {}");
        let struc = prog.struct0();
        assert_eq!(0, struc.fields.len());
        assert_eq!("Foo", *interner.str(struc.name));
    }

    #[test]
    fn parse_struct_one_field() {
        let (prog, interner) = parse(
            "struct Bar {
            f1: Foo1,
        }",
        );
        let struc = prog.struct0();
        assert_eq!(1, struc.fields.len());
        assert_eq!("Bar", *interner.str(struc.name));

        let f1 = &struc.fields[0];
        assert_eq!("f1", *interner.str(f1.name));
    }

    #[test]
    fn parse_struct_multiple_fields() {
        let (prog, interner) = parse(
            "struct FooBar {
            fa: Foo1,
            fb: Foo2,
        }",
        );
        let struc = prog.struct0();
        assert_eq!(2, struc.fields.len());
        assert_eq!("FooBar", *interner.str(struc.name));

        let f1 = &struc.fields[0];
        assert_eq!("fa", *interner.str(f1.name));

        let f2 = &struc.fields[1];
        assert_eq!("fb", *interner.str(f2.name));
    }

    #[test]
    fn parse_struct_with_type_params() {
        let (prog, interner) = parse(
            "struct Bar[T1, T2] {
            f1: T1, f2: T2,
        }",
        );
        let xstruct = prog.struct0();
        assert_eq!(2, xstruct.fields.len());
        assert_eq!("Bar", *interner.str(xstruct.name));

        assert_eq!(2, xstruct.type_params.as_ref().unwrap().len());
    }

    #[test]
    fn parse_struct_lit_while() {
        let stmt = parse_stmt("while i < n { }");
        let while_stmt = stmt.to_while().unwrap();
        let bin = while_stmt.cond.to_bin().unwrap();

        assert!(bin.lhs.is_ident());
        assert!(bin.rhs.is_ident());
    }

    #[test]
    fn parse_struct_lit_if() {
        let (expr, _) = parse_expr("if i < n { }");
        let ifexpr = expr.to_if().unwrap();
        let bin = ifexpr.cond.to_bin().unwrap();

        assert!(bin.lhs.is_ident());
        assert!(bin.rhs.is_ident());
    }

    #[test]
    fn parse_lit_float() {
        let (expr, _) = parse_expr("1.2");

        let lit = expr.to_lit_float().unwrap();

        assert_eq!(1.2, lit.value);
    }

    #[test]
    fn parse_template() {
        let (expr, _) = parse_expr("\"a${1}b${2}c\"");
        let tmpl = expr.to_template().unwrap();
        assert_eq!(tmpl.parts.len(), 5);

        assert_eq!("a".to_string(), tmpl.parts[0].to_lit_str().unwrap().value);
        assert_eq!(1, tmpl.parts[1].to_lit_int().unwrap().value);
        assert_eq!("b".to_string(), tmpl.parts[2].to_lit_str().unwrap().value);
        assert_eq!(2, tmpl.parts[3].to_lit_int().unwrap().value);
        assert_eq!("c".to_string(), tmpl.parts[4].to_lit_str().unwrap().value);

        let (expr, _) = parse_expr("\"a\\${1}b\"");
        assert!(expr.is_lit_str());
    }

    #[test]
    fn parse_class_type_params() {
        let (prog, interner) = parse("class Foo[T]");
        let cls = prog.cls0();

        let type_params = cls.type_params.as_ref().unwrap();
        assert_eq!(1, type_params.len());
        assert_eq!("T", *interner.str(type_params[0].name));

        let (prog, interner) = parse("class Foo[X]");
        let cls = prog.cls0();

        let type_params = cls.type_params.as_ref().unwrap();
        assert_eq!(1, type_params.len());
        assert_eq!("X", *interner.str(type_params[0].name));
    }

    #[test]
    fn parse_multiple_class_type_params() {
        let (prog, interner) = parse("class Foo[A, B]");
        let cls = prog.cls0();

        let type_params = cls.type_params.as_ref().unwrap();
        assert_eq!(2, type_params.len());
        assert_eq!("A", *interner.str(type_params[0].name));
        assert_eq!("B", *interner.str(type_params[1].name));
    }

    #[test]
    fn parse_empty_trait() {
        let (prog, interner) = parse("trait Foo { }");
        let trait_ = prog.trait0();

        assert_eq!("Foo", *interner.str(trait_.name));
        assert_eq!(0, trait_.methods.len());
    }

    #[test]
    fn parse_trait_with_function() {
        let (prog, interner) = parse("trait Foo { fn empty(); }");
        let trait_ = prog.trait0();

        assert_eq!("Foo", *interner.str(trait_.name));
        assert_eq!(1, trait_.methods.len());
        assert_eq!(false, trait_.methods[0].is_static);
    }

    #[test]
    fn parse_trait_with_static_function() {
        let (prog, interner) = parse("trait Foo { @static fn empty(); }");
        let trait_ = prog.trait0();

        assert_eq!("Foo", *interner.str(trait_.name));
        assert_eq!(1, trait_.methods.len());
        assert_eq!(true, trait_.methods[0].is_static);
    }

    #[test]
    fn parse_empty_impl() {
        let (prog, interner) = parse("impl Foo for A {}");
        let impl_ = prog.impl0();

        assert_eq!(
            "Foo",
            impl_.trait_type.as_ref().unwrap().to_string(&interner)
        );
        assert_eq!("A", impl_.extended_type.to_string(&interner));
        assert_eq!(0, impl_.methods.len());
    }

    #[test]
    fn parse_impl_with_function() {
        let (prog, interner) = parse("impl Bar for B { fn foo(); }");
        let impl_ = prog.impl0();

        assert_eq!(
            "Bar",
            impl_.trait_type.as_ref().unwrap().to_string(&interner)
        );
        assert_eq!("B", impl_.extended_type.to_string(&interner));
        assert_eq!(1, impl_.methods.len());
        assert_eq!(false, impl_.methods[0].is_static);
    }

    #[test]
    fn parse_impl_with_static_function() {
        let (prog, interner) = parse("impl Bar for B { @static fn foo(); }");
        let impl_ = prog.impl0();

        assert_eq!(
            "Bar",
            impl_.trait_type.as_ref().unwrap().to_string(&interner)
        );
        assert_eq!("B", impl_.extended_type.to_string(&interner));
        assert_eq!(1, impl_.methods.len());
        assert_eq!(true, impl_.methods[0].is_static);
    }

    #[test]
    fn parse_global_var() {
        let (prog, interner) = parse("var a: int = 0;");
        let global = prog.global0();

        assert_eq!("a", *interner.str(global.name));
        assert_eq!(true, global.mutable);
    }

    #[test]
    fn parse_global_let() {
        let (prog, interner) = parse("let b: int = 0;");
        let global = prog.global0();

        assert_eq!("b", *interner.str(global.name));
        assert_eq!(false, global.mutable);
    }

    #[test]
    fn parse_lit_char() {
        let (expr, _) = parse_expr("'a'");
        let lit = expr.to_lit_char().unwrap();

        assert_eq!('a', lit.value);
    }

    #[test]
    fn parse_fct_call_with_type_param() {
        let (expr, _) = parse_expr("Array[Int]()");
        let call = expr.to_call().unwrap();
        let type_params = call.callee.to_type_param().unwrap();

        assert_eq!(1, type_params.args.len());

        let (expr, _) = parse_expr("Foo[Int, Long]()");
        let call = expr.to_call().unwrap();
        let type_params = call.callee.to_type_param().unwrap();

        assert_eq!(2, type_params.args.len());

        let (expr, _) = parse_expr("Bar[]()");
        let call = expr.to_call().unwrap();
        let type_params = call.callee.to_type_param().unwrap();

        assert_eq!(0, type_params.args.len());

        let (expr, _) = parse_expr("Vec()");
        let call = expr.to_call().unwrap();

        assert!(call.callee.is_ident());
    }

    #[test]
    fn parse_static_method() {
        let (prog, _) = parse(
            "class A {
                @static fn test() {}
              }",
        );
        let cls = prog.cls0();
        assert_eq!(1, cls.methods.len());

        let mtd = &cls.methods[0];
        assert!(mtd.is_static);
    }

    #[test]
    fn parse_call_with_path() {
        let (expr, _) = parse_expr("Foo::get()");
        let call = expr.to_call().unwrap();

        assert!(call.callee.is_path());
        assert_eq!(0, call.args.len());
    }

    #[test]
    fn parse_fct_with_type_params() {
        let (prog, _) = parse("fn f[T]() {}");
        let fct = prog.fct0();

        assert_eq!(1, fct.type_params.as_ref().unwrap().len());
    }

    #[test]
    fn parse_const() {
        let (prog, interner) = parse("const x: int = 0;");
        let const_ = prog.const0();

        assert_eq!("x", *interner.str(const_.name));
    }

    #[test]
    fn parse_generic_with_bound() {
        let (prog, _) = parse("class A[T: Foo]");
        let cls = prog.cls0();

        let type_param = &cls.type_params.as_ref().unwrap()[0];
        assert_eq!(1, type_param.bounds.len());
    }

    #[test]
    fn parse_generic_with_multiple_bounds() {
        let (prog, _) = parse("class A[T: Foo + Bar]");
        let cls = prog.cls0();

        let type_param = &cls.type_params.as_ref().unwrap()[0];
        assert_eq!(2, type_param.bounds.len());
    }

    #[test]
    fn parse_generic_super_class() {
        let (prog, _) = parse("class A: B[SomeType, SomeOtherType]");
        let cls = prog.cls0();

        assert!(cls.parent_class.is_some());
    }

    #[test]
    fn parse_generic_super_class_with_nested_type_definition() {
        let (prog, _) = parse("class A: B[SomeType[SomeOtherType[Int]]]");
        let cls = prog.cls0();

        assert!(cls.parent_class.is_some());
    }

    #[test]
    fn parse_lambda_no_params_no_return_value() {
        let (expr, _) = parse_expr("|| {}");
        let lambda = expr.to_lambda().unwrap();

        assert!(lambda.return_type.is_none());
    }

    #[test]
    fn parse_lambda_no_params_unit_as_return_value() {
        let (expr, _) = parse_expr("|| : () {}");
        let lambda = expr.to_lambda().unwrap();
        let ret = lambda.return_type.as_ref().unwrap();

        assert!(ret.is_unit());
    }

    #[test]
    fn parse_lambda_no_params_with_return_value() {
        let (expr, interner) = parse_expr("||: A {}");
        let lambda = expr.to_lambda().unwrap();
        let ret = lambda.return_type.as_ref().unwrap();
        let basic = ret.to_basic().unwrap();

        assert_eq!("A", *interner.str(basic.name()));
    }

    #[test]
    fn parse_lambda_with_one_param() {
        let (expr, interner) = parse_expr("|a: A|: B {}");
        let lambda = expr.to_lambda().unwrap();

        assert_eq!(1, lambda.params.len());

        let param = &lambda.params[0];
        assert_eq!("a", *interner.str(param.name));
        let basic = param.data_type.to_basic().unwrap();
        assert_eq!("A", *interner.str(basic.name()));

        let ret = lambda.return_type.as_ref().unwrap();
        let basic = ret.to_basic().unwrap();

        assert_eq!("B", *interner.str(basic.name()));
    }

    #[test]
    fn parse_lambda_with_two_params() {
        let (expr, interner) = parse_expr("|a: A, b: B|: C {}");
        let lambda = expr.to_lambda().unwrap();

        assert_eq!(2, lambda.params.len());

        let param = &lambda.params[0];
        assert_eq!("a", *interner.str(param.name));
        let basic = param.data_type.to_basic().unwrap();
        assert_eq!("A", *interner.str(basic.name()));

        let param = &lambda.params[1];
        assert_eq!("b", *interner.str(param.name));
        let basic = param.data_type.to_basic().unwrap();
        assert_eq!("B", *interner.str(basic.name()));

        let ret = lambda.return_type.as_ref().unwrap();
        let basic = ret.to_basic().unwrap();

        assert_eq!("C", *interner.str(basic.name()));
    }

    #[test]
    fn parse_for() {
        let stmt = parse_stmt("for i in a+b {}");
        assert!(stmt.is_for());
    }

    #[test]
    fn parse_new_call_ident() {
        let (expr, _interner) = parse_expr("i");
        assert!(expr.is_ident());
    }

    #[test]
    fn parse_new_call_path() {
        let (expr, _interner) = parse_expr("Foo::bar");
        let path = expr.to_path().unwrap();
        assert!(path.lhs.is_ident());
        assert!(path.rhs.is_ident());
    }

    #[test]
    fn parse_new_call_call() {
        let (expr, _interner) = parse_expr("foo(1,2)");
        let call = expr.to_call().unwrap();
        assert!(call.callee.is_ident());
        assert_eq!(call.args.len(), 2);
    }

    #[test]
    fn parse_block() {
        let (expr, _) = parse_expr("{1}");
        assert!(expr.to_block().unwrap().expr.as_ref().unwrap().is_lit_int());

        let (expr, _) = parse_expr("({}) + 1");
        assert!(expr.is_bin());

        let (expr, _) = parse_expr("1 + {}");
        assert!(expr.is_bin());
    }

    #[test]
    fn parse_if_expr() {
        parse_err(
            "fn f() { if true { 1 } else { 2 } * 4 }",
            ParseError::ExpectedFactor("*".into()),
            1,
            35,
        );
    }

    #[test]
    fn parse_tuple() {
        let (expr, _) = parse_expr("(1,)");
        assert_eq!(expr.to_tuple().unwrap().values.len(), 1);

        let (expr, _) = parse_expr("(1)");
        assert!(expr.is_paren());

        let (expr, _) = parse_expr("(1,2,3)");
        assert_eq!(expr.to_tuple().unwrap().values.len(), 3);

        let (expr, _) = parse_expr("(1,2,3,4,)");
        assert_eq!(expr.to_tuple().unwrap().values.len(), 4);
    }

    #[test]
    fn parse_enum() {
        let (prog, _) = parse("enum Foo { A, B, C }");
        let enum_ = prog.enum0();
        assert_eq!(enum_.variants.len(), 3);
    }

    #[test]
    fn parse_enum_with_type_params() {
        let (prog, _) = parse("enum MyOption[T] { None, Some(T), }");
        let enum_ = prog.enum0();
        assert_eq!(enum_.variants.len(), 2);
        assert!(enum_.variants[0].types.is_none());
        assert_eq!(enum_.variants[1].types.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn parse_alias() {
        let (prog, _) = parse("alias NewType = Int;");
        let _alias = prog.alias0();
    }

    #[test]
    fn parse_module() {
        let (prog, _) = parse("mod foo { fn bar() {} fn baz() {} }");
        let module = prog.module0();
        let elements = module.elements.as_ref().unwrap();
        assert_eq!(elements.len(), 2);
        assert!(elements[0].to_function().is_some());
        assert!(elements[1].to_function().is_some());
    }

    #[test]
    fn parse_mod_without_body() {
        let (prog, _) = parse("mod foo;");
        let module = prog.module0();
        assert!(module.elements.is_none());
    }

    #[test]
    fn parse_match() {
        parse_expr("match x { }");
        parse_expr("match x { A(x, b) => 1, B => 2 }");
        parse_expr("match x { A(x, b) => 1, B | C => 2 }");
    }

    #[test]
    fn parse_use_declaration() {
        parse_err(
            "use foo::bar{a, b, c}",
            ParseError::ExpectedToken(";".into(), "{".into()),
            1,
            13,
        );

        parse_err(
            "use ::foo;",
            ParseError::ExpectedIdentifier("::".into()),
            1,
            5,
        );
    }
}
