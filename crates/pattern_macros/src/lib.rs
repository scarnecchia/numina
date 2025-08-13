use darling::{FromDeriveInput, FromField};
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Ident, Type};

/// Attributes for the Entity derive macro
#[derive(Debug, FromDeriveInput)]
#[darling(attributes(entity), forward_attrs(allow, doc, cfg))]
struct EntityOpts {
    /// The table name (defaults to lowercase struct name)
    #[darling(default)]
    table: Option<String>,

    /// The entity type (user, agent, task, memory, event)
    entity_type: String,

    /// The crate path to use (defaults to "crate" for internal use, "::pattern_core" for external)
    #[darling(default)]
    crate_path: Option<String>,

    /// Whether this is an edge entity (for SurrealDB RELATE operations)
    #[darling(default)]
    edge: bool,
}

/// Field-level attributes
#[derive(Debug, Default, FromField)]
#[darling(attributes(entity))]
struct FieldOpts {
    /// Skip this field when storing to database
    #[darling(default)]
    skip: bool,

    /// Store as a different type in the database
    #[darling(default)]
    db_type: Option<String>,

    /// This field represents a relation to another table
    #[darling(default)]
    relation: Option<String>,

    /// This field uses a custom edge entity for the relation
    #[darling(default)]
    edge_entity: Option<String>,
}

/// Derive macro for database entities
///
/// This macro generates:
/// 1. A storage struct with SurrealDB types
/// 2. Conversions between domain and storage types
/// 3. DbEntity trait implementation
///
/// Example:
/// ```
/// #[derive(Entity)]
/// #[entity(entity_type = "user")]
/// struct User {
///     pub id: UserId,
///     pub discord_id: Option<String>,
///     pub created_at: DateTime<Utc>,
///     pub updated_at: DateTime<Utc>,
///
///     // Simple relation
///     #[entity(relation = "owns")]
///     pub owned_agents: Vec<AgentId>,
///
///     // Relation with custom edge entity
///     #[entity(edge_entity = "UserTaskAssignment")]
///     pub assigned_tasks: Vec<Task>,
/// }
/// ```
#[proc_macro_derive(Entity, attributes(entity))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let opts = match EntityOpts::from_derive_input(&input) {
        Ok(v) => v,
        Err(e) => return TokenStream::from(e.write_errors()),
    };

    let name = &input.ident;
    let db_model_name = Ident::new(&format!("{name}DbModel"), name.span());
    let entity_type = &opts.entity_type;
    let table_name = opts.table.unwrap_or_else(|| {
        // Special case for message entity - use "msg" as table name
        if entity_type == "message" {
            "msg".to_string()
        } else {
            entity_type.to_string()
        }
    });

    // Determine crate path - default to "crate" if not specified
    let crate_path_str = opts.crate_path.unwrap_or_else(|| "crate".to_string());
    let crate_path: syn::Path = syn::parse_str(&crate_path_str).expect("Invalid crate path");

    // Extract fields
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("Entity can only be derived for structs with named fields"),
        },
        _ => panic!("Entity can only be derived for structs"),
    };

    // Check if this is an edge entity
    let is_edge_entity = opts.edge;

    // Generate field lists for domain and storage structs
    let mut storage_fields = vec![];
    let mut storage_field_names: Vec<proc_macro2::TokenStream> = vec![];
    let mut to_storage_conversions = vec![];
    let mut from_storage_conversions = vec![];
    let mut skip_fields = vec![];
    let mut relation_fields = vec![];
    let mut edge_entity_fields = vec![];
    let mut field_definitions = vec![];

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let field_opts = FieldOpts::from_field(field).unwrap_or_default();

        // Skip fields don't go in storage struct
        if field_opts.skip {
            skip_fields.push((field_name, field_type));
            continue;
        }

        // Check if this field has edge_entity attribute (for tuple relations with metadata)
        // YES THIS LOOKS WEIRD AND REDUNDANT. DO NOT CHANGE, IT BREAKS THE MACRO!!!!
        if let (Some(relation_name), Some(edge_entity)) =
            (&field_opts.relation, &field_opts.edge_entity)
        {
            // Edge entity relation - the edge_entity value is the relation table name
            edge_entity_fields.push((
                field_name,
                field_type,
                relation_name.clone(),
                edge_entity.clone(),
            ));
            // Edge relations are not stored in the main table
            continue;
        } else if let Some(relation_name) = field_opts.relation {
            // Regular relation fields are stored in separate tables
            relation_fields.push((field_name, field_type, relation_name));
            // Relations are not stored in the main table
            continue;
        }

        // Determine storage type based on entity type and field name
        let storage_type = determine_storage_type(entity_type, field_name, field_type, &field_opts);

        // Add serde rename attributes for edge entity in/out fields
        let field_def = if is_edge_entity && (field_name == "in_id" || field_name == "out_id") {
            let rename = if field_name == "in_id" { "in" } else { "out" };
            quote! {
                #[serde(rename = #rename)]
                pub #field_name: #storage_type
            }
        } else {
            quote! { pub #field_name: #storage_type }
        };

        storage_fields.push(field_def);
        storage_field_names.push(quote! { stringify!(#field_name).to_string() });

        // Generate field definition for schema
        let field_def =
            generate_field_definition(field_name, &storage_type, &table_name, &field_opts);
        field_definitions.push(field_def);

        // Generate conversions - check if we need custom conversion
        let needs_custom_conversion =
            field_opts.db_type.is_some() && !matches_type(&storage_type, field_type);

        to_storage_conversions.push(generate_to_storage(
            field_name,
            field_type,
            &storage_type,
            needs_custom_conversion,
        ));
        from_storage_conversions.push(generate_from_storage(
            field_name,
            field_type,
            &storage_type,
            &crate_path,
            needs_custom_conversion,
            entity_type,
            is_edge_entity,
        ));
    }

    // Skip fields need to be handled in from_storage (reconstructed from other data)
    for (field_name, field_type) in &skip_fields {
        // Skip fields are not stored, so they need custom reconstruction logic
        let default_value = if is_id_type(field_type) {
            quote! { #field_type::nil() }
        } else {
            quote! { Default::default() }
        };
        from_storage_conversions.push(quote! {
            #field_name: #default_value
        });
    }

    // Edge entity fields are loaded separately, so default them for now

    for (field_name, field_type, _relation_name, _edge_entity) in &edge_entity_fields {
        // For edge entity fields, we need to handle the full type properly
        // Just use the field type directly with turbofish syntax
        let default_value = quote! { <#field_type>::default() };

        from_storage_conversions.push(quote! {
            #field_name: #default_value
        });
    }

    // Relation fields are loaded separately, so default them for now
    for (field_name, field_type, _relation_name) in &relation_fields {
        let default_value = if is_vec_type(field_type) {
            let inner_type =
                extract_inner_type(field_type).expect("Vec type should have inner type");
            // Always use explicit type annotation for Vec
            quote! { Vec::<#inner_type>::new() }
        } else if is_option_type(field_type) {
            let inner_type =
                extract_inner_type(field_type).expect("Option type should have inner type");
            if is_id_type(inner_type) {
                quote! { None }
            } else {
                quote! { None }
            }
        } else if is_id_type(field_type) {
            quote! { #field_type::nil() }
        } else if is_option_type(field_type) {
            quote! { None }
        } else {
            quote! { Default::default() }
        };
        from_storage_conversions.push(quote! {
            #field_name: #default_value
        });
    }

    // Generate relation table definitions
    for (_field_name, _field_type, relation_name) in &relation_fields {
        field_definitions.push(format!("DEFINE TABLE OVERWRITE {relation_name} SCHEMALESS"));
    }

    // Generate relation table definitions
    for (_field_name, _field_type, relation_name, _edge_entity) in &edge_entity_fields {
        field_definitions.push(format!("DEFINE TABLE OVERWRITE {relation_name} SCHEMALESS"));
    }

    // Extract the id field type
    let id_field = fields
        .iter()
        .find(|f| f.ident.as_ref().map(|i| i == "id").unwrap_or(false))
        .expect("Entity must have an 'id' field");

    let id_field_type = &id_field.ty;

    // Generate the ID type based on entity type or extract from Id<T>
    let id_type = if is_edge_entity {
        // For edge entities, we'll handle this specially
        // Use a dummy type that won't be used in practice
        quote! { #crate_path::id::RelationId }
    } else {
        match entity_type.as_str() {
            "user" => quote! { #crate_path::id::UserId },
            "agent" => quote! { #crate_path::id::AgentId },
            "memory" => quote! { #crate_path::id::MemoryId },
            "message" => quote! { #crate_path::id::MessageId },
            "event" => quote! { #crate_path::id::EventId },
            _ => {
                // For custom entity types, we need to determine the IdType
                // The id field could be:
                // 1. Id<SomeIdType> - direct type with angle brackets
                // 2. AgentId - type alias for Id<AgentIdType>
                // 3. RelationId - type alias for Id<RelationIdType>

                // For type aliases, we can't see the inner type directly
                // So we'll use a naming convention: if it ends with "Id",
                // assume the inner type is the same name + "Type"
                if let syn::Type::Path(type_path) = id_field_type {
                    if let Some(segment) = type_path.path.segments.last() {
                        let type_name = segment.ident.to_string();

                        if type_name.ends_with("Id") {
                            // Type alias like AgentId -> AgentIdType
                            let base_name = &type_name[..type_name.len() - 2];
                            let id_type_name = format!("{base_name}Id");
                            let id_type_ident =
                                syn::Ident::new(&id_type_name, segment.ident.span());
                            quote! { #id_type_ident }
                        } else {
                            // Unknown pattern, use the type as is
                            quote! { #id_field_type }
                        }
                    } else {
                        quote! { #id_field_type }
                    }
                } else {
                    quote! { #id_field_type }
                }
            }
        }
    };

    // Generate helper function name
    let helper_fn = Ident::new(&format!("generate_{entity_type}_schema"), name.span());

    // Generate field keys function name
    let field_keys_fn = Ident::new(&format!("{entity_type}_field_keys"), name.span());

    // Generate store_relations method
    let store_relation_calls = relation_fields.iter().map(|(field_name, field_type, relation_name)| {
        let is_vec = is_vec_type(field_type);
        let is_id = is_id_type(field_type);

        if is_vec {
            // Extract inner type from Vec<T>
            let inner_type = extract_inner_type(field_type).expect("Vec type should have inner type");
            let inner_is_id = is_id_type(inner_type);

            if inner_is_id {
                // Vec<ID> - just store the relations
                quote! {
                    // Store Vec<ID> relations
                    for related_id in &self.#field_name {
                        let query = format!(
                            "RELATE {}->{}->{} SET created_at = time::now()",
                            ::surrealdb::RecordId::from(self.id.clone()), #relation_name,
                            ::surrealdb::RecordId::from(related_id)
                        );
                        db.query(&query).await?;
                    }
                }
            } else {
                // Vec<Entity> - upsert entities and create relations
                quote! {
                    // Store Vec<Entity> relations - first upsert each entity, then create relations
                    for related_entity in &self.#field_name {

                        let db_model = related_entity.to_db_model();
                        // Upsert the related entity
                        tracing::trace!("upserting: {:?}", db_model);
                        let e: Option<<#inner_type as #crate_path::db::entity::DbEntity>::DbModel> = db
                            .upsert(db_model.id.clone())
                            .content(db_model)
                            .await?;

                        tracing::trace!("upserted: {:?}", e);

                        // Create the relation
                        let query = format!(
                            "RELATE {}->{}->{} SET created_at = time::now()",
                            ::surrealdb::RecordId::from(self.id.clone()), #relation_name,
                            ::surrealdb::RecordId::from(related_entity.id().clone())
                        );
                        db.query(&query).await?;
                    }
                }
            }
        } else if is_id {
            // Single ID relation
            quote! {
                // Store single ID relation
                if !self.#field_name.is_nil() {
                    let query = format!(
                        "RELATE {}->{}->{} SET created_at = time::now()",
                        ::surrealdb::RecordId::from(self.id.clone()), #relation_name,
                        ::surrealdb::RecordId::from(self.#field_name)
                    );
                    db.query(&query).await?;
                }
            }
        } else {
            // Single Entity relation - check if it's Option<Entity> or just Entity
            let is_option = is_option_type(field_type);
            if is_option {
                quote! {
                    // Store single Option<Entity> relation
                    if let Some(related_entity) = &self.#field_name {
                        // Upsert the related entity
                        let inner_type_name = stringify!(#field_type).trim_start_matches("Option < ").trim_end_matches(" >");
                        let db_model = related_entity.to_db_model();
                        let e: Option<<#field_type as #crate_path::db::entity::DbEntity>::DbModel> = db
                            .upsert(db_model.id.clone())
                            .content(db_model)
                            .await
                            ?;

                        tracing::trace!("upserted: {:?}", e);
                        // Create the relation
                        let query = format!(
                            "RELATE {}->{}->{} SET created_at = time::now()",
                            ::surrealdb::RecordId::from(self.id.clone()), #relation_name,
                            ::surrealdb::RecordId::from(related_entity.id().clone())
                        );
                        db.query(&query).await?;
                    }
                }
            } else {
                quote! {
                    // Store single Entity relation (non-Option)
                    // Upsert the related entity
                    let db_model = self.#field_name.to_db_model();
                    let e: Option<<#field_type as #crate_path::db::entity::DbEntity>::DbModel> = db
                        .upsert(db_model.id.clone())
                        .content(db_model)
                        .await
                        ?;

                    tracing::trace!("upserted: {:?}", e);

                    // Create the relation
                    let query = format!(
                        "RELATE {}->{}->{} SET created_at = time::now()",
                        ::surrealdb::RecordId::from(self.id.clone()), #relation_name,
                        ::surrealdb::RecordId::from(self.#field_name.id().clone())
                    );
                    db.query(&query).await?;
                }
            }
        }
    });

    // Generate load_relations method - need to use entity instead of self for the closures
    let load_relation_calls = relation_fields.iter().map(|(field_name, field_type, relation_name)| {
        let is_vec = is_vec_type(field_type);
        let is_id = is_id_type(field_type);

        if is_vec {
            let inner_type = extract_inner_type(field_type).expect("Vec type should have inner type");
            let inner_is_id = is_id_type(inner_type);

            if inner_is_id {
                // Vec<ID> - just load the IDs
                quote! {
                    // Load Vec<ID> relations
                    let query = format!("SELECT id, ->{}->{} AS related_entitites FROM $parent ORDER BY id ASC",
                        #relation_name,
                        Self::related_table_from_id_type(stringify!(#inner_type)));

                    tracing::trace!("id vec query: {}", query);
                    let mut result = db.query(&query)
                        .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                        .await?;

                    tracing::trace!("vec result {:?}", result);

                    let db_models: Vec<Vec<::surrealdb::RecordId>> =
                        result.take("related_entitites")?;

                    tracing::trace!("vec db models: {:?}", db_models);

                    // Convert from db models to domain models
                    self.#field_name = db_models.concat().into_iter()
                        .map(|record_id| #inner_type::from_record(record_id) )
                        .collect();
                }
            } else {
                // Vec<Entity> - fetch full entities
                quote! {
                    // Load Vec<Entity> relations - fetch full entities
                    let query = format!("SELECT id, ->{}->{}[*] AS related_entitites FROM $parent ORDER BY id ASC",
                        #relation_name,
                        Self::related_table_from_type(stringify!(#inner_type)));

                    tracing::trace!("full vec query: {}", query);

                    let mut result = db.query(&query)
                        .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                        .await?;

                    tracing::trace!("vec result {:?}", result);

                    let db_models: Vec<Vec<<#inner_type as #crate_path::db::entity::DbEntity>::DbModel>> =
                        result.take("related_entitites")?;

                    tracing::trace!("vec db models: {:?}", db_models);

                    // Convert from db models to domain models
                    self.#field_name = db_models.concat().into_iter()
                        .map(|db_model| <#inner_type as #crate_path::db::entity::DbEntity>::from_db_model(db_model))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert relation: {:?}", e)))
                        ))?;

                    tracing::trace!("object: {:?}", self);
                }
            }
        } else if is_id {
            // Single ID relation
            // Load single ID relation
            quote! {
                // Load single ID relation
                let query = format!("SELECT id, ->{}->{} AS related_entity FROM $parent ORDER BY id ASC LIMIT 1",
                    #relation_name,
                    Self::related_table_from_id_type(stringify!(#field_type)));

                tracing::trace!("single id query: {}", query);
                let mut result = db.query(&query)
                    .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                    .await?;

                let record_ids: Vec<Vec<::surrealdb::RecordId>> =
                    result.take("related_entity")?;

                self.#field_name = record_ids.concat().into_iter().next()
                    .map(|record_id| #field_type::from_record(record_id))
                    .unwrap_or_else(|| #field_type::nil().simple());
            }
        } else {
            // Single Entity relation - check if it's Option<Entity> or just Entity
            let is_option = is_option_type(field_type);
            if is_option {
                let inner_type = extract_inner_type(field_type).expect("Option type should have inner type");
                quote! {
                    // Load single Option<Entity> relation - fetch full entity
                    let query = format!("SELECT id, ->{}->{}[*] AS related_entity FROM $parent ORDER BY id ASC LIMIT 1",
                        #relation_name,
                        Self::related_table_from_type(stringify!(#inner_type)));

                    let mut result = db.query(&query)
                        .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                        .await?;

                    let db_models: Vec<Vec<<#inner_type as #crate_path::db::entity::DbEntity>::DbModel>> =
                        result.take("related_entity")?;

                    // Convert from db model to domain model
                    self.#field_name = if let Some(db_model) = db_models.concat().into_iter().next() {
                        Some(<#inner_type as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                            .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert relation: {:?}", e)))
                            ))?)
                    } else {
                        None
                    };
                }
            } else {
                quote! {
                    // Load single Entity relation (non-Option) - fetch full entity
                    let query = format!("SELECT id, ->{}->{}[*] AS related_entity FROM $parent ORDER BY id ASC LIMIT 1",
                        #relation_name,
                        Self::related_table_from_type(stringify!(#field_type)));

                    let mut result = db.query(&query)
                        .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                        .await?;

                    let db_models: Vec<Vec<<#field_type as #crate_path::db::entity::DbEntity>::DbModel>> =
                        result.take("related_entity")?;

                    // Convert from db model to domain model
                    self.#field_name = if let Some(db_model) = db_models.concat().into_iter().next() {
                        <#field_type as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                            .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert relation: {:?}", e)))
                            ))?
                    } else {
                        return Err(#crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                format!("Required relation {} not found", stringify!(#field_name))
                            ))
                        ));
                    };
                }
            }
        }
    });

    // Generate store calls for edge entity relations
    let store_edge_entity_calls = edge_entity_fields.iter().map(|(field_name, field_type, _relation_name, _edge_entity)| {
        let is_vec = is_vec_type(field_type);

        if is_vec {
            // Check if inner type is a tuple
            let inner_type = extract_inner_type(field_type).expect("Vec type should have inner type");
            if is_tuple_type(inner_type) {
                // Vec<(Entity, EdgeEntity)> with edge entity
                quote! {
                    // Store Vec<(Entity, EdgeEntity)> with edge entity relations
                    for (related_entity, edge_data) in &self.#field_name {
                        // First upsert the related entity using its DbEntity implementation
                        let related_id = related_entity.id().clone();
                        let db_model = related_entity.to_db_model();
                        let _stored = db
                            .upsert(related_id.to_record_id())
                            .content(db_model)
                            .await
                            ?;

                        // Use create_relation_typed to store the edge entity
                        let _edge_stored = #crate_path::db::ops::create_relation_typed(db, edge_data).await
                            .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    format!("Failed to create edge relation: {:?}", e)
                                ))
                            ))?;
                    }
                }
            } else {
                // Vec<EdgeEntity> without tuple
                quote! {
                    // Store Vec<EdgeEntity> relations
                    for edge_data in &self.#field_name {
                        // Use create_relation_typed to store the edge entity
                        let _edge_stored = #crate_path::db::ops::create_relation_typed(db, edge_data).await
                            .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    format!("Failed to create edge relation: {:?}", e)
                                ))
                            ))?;
                    }
                }
            }
        } else if is_option_type(field_type) {
            // Check if inner type is a tuple
            let inner_type = extract_inner_type(field_type).expect("Option type should have inner type");
            if is_tuple_type(inner_type) {
                // Option<(Entity, EdgeEntity)> with edge entity
                quote! {
                    // Store Option<(Entity, EdgeEntity)> with edge entity relation
                    if let Some((related_entity, edge_data)) = &self.#field_name {
                        // First upsert the related entity
                        let db_model = related_entity.to_db_model();
                        let _stored = db
                            .upsert(db_model.id.clone())
                            .content(db_model)
                            .await
                            ?;

                        // Use create_relation_typed to store the edge entity
                        let _edge_stored = #crate_path::db::ops::create_relation_typed(db, edge_data).await
                            .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    format!("Failed to create edge relation: {:?}", e)
                                ))
                            ))?;
                    }
                }
            } else {
                // Option<EdgeEntity> without tuple
                quote! {
                    // Store Option<EdgeEntity> relation
                    if let Some(edge_data) = &self.#field_name {
                        // Use create_relation_typed to store the edge entity
                        let _edge_stored = #crate_path::db::ops::create_relation_typed(db, edge_data).await
                            .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    format!("Failed to create edge relation: {:?}", e)
                                ))
                            ))?;
                    }
                }
            }
        } else {
            // Check if the field is a tuple type
            if is_tuple_type(field_type) {
                // Single (Entity, EdgeEntity) with edge entity
                quote! {
                    // Store single (Entity, EdgeEntity) with edge entity relation
                    let (related_entity, edge_data) = &self.#field_name;
                    let db_model = related_entity.to_db_model();
                    let _stored = db
                        .upsert(db_model.id.clone())
                        .content(db_model)
                        .await
                        ?;

                    // Use create_relation_typed to store the edge entity
                    let _edge_stored = #crate_path::db::ops::create_relation_typed(db, edge_data).await
                        .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                format!("Failed to create edge relation: {:?}", e)
                            ))
                        ))?;
                }
            } else {
                // Single EdgeEntity without tuple
                quote! {
                    // Store single EdgeEntity relation
                    let _edge_stored = #crate_path::db::ops::create_relation_typed(db, &self.#field_name).await
                        .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                format!("Failed to create edge relation: {:?}", e)
                            ))
                        ))?;
                }
            }
        }
    });

    // Generate load calls for edge entity relations
    let load_edge_entity_calls = edge_entity_fields.iter().map(|(field_name, field_type, relation_name, edge_entity_type)| {
        let is_vec = is_vec_type(field_type);

        if is_vec {
            // For edge entity relations, we should use the actual type from the field
            // instead of trying to construct it from a string
            // Extract the tuple types directly from the field type
            if let Some((entity_type, edge_type)) = extract_tuple_types_from_container(field_type) {
                quote! {
                    // Load Vec<(Entity, EdgeEntity)> with edge entity relations
                    // Query the edge entities - need to check if this is group_members which has reversed in/out
                    let query = if #relation_name == "group_members" {
                        format!("SELECT * FROM {} WHERE out = $parent ORDER BY id ASC", #relation_name)
                    } else {
                        format!("SELECT * FROM {} WHERE in = $parent ORDER BY id ASC", #relation_name)
                    };

                    tracing::info!("Loading edge entities with query: {}, parent: {:?}", query, self.id);

                    let mut result = db.query(&query)
                        .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                        .await?;

                    // Take the edge DB models directly
                    let edge_db_models: Vec<<#edge_type as #crate_path::db::entity::DbEntity>::DbModel> = result.take(0)?;

                    tracing::info!("Found {} {} relations", edge_db_models.len(), #relation_name);

                    // Convert DB models to domain types
                    let edge_entities: Vec<#edge_type> = edge_db_models
                        .into_iter()
                        .map(|db_model| <#edge_type as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                            .map_err(#crate_path::db::DatabaseError::from))
                        .collect::<Result<Vec<_>, _>>()?;

                    // Now fetch the related entities
                    let mut entities = Vec::<(#entity_type, #edge_type)>::new();

                    for edge in edge_entities {
                        // Get the related entity - for group_members we need in_id (agent), otherwise out_id
                        let related_id = if #relation_name == "group_members" {
                            ::surrealdb::RecordId::from(&edge.in_id)
                        } else {
                            ::surrealdb::RecordId::from(&edge.out_id)
                        };

                        let related_db: Option<<#entity_type as #crate_path::db::entity::DbEntity>::DbModel> =
                            db.select(related_id).await?;

                        if let Some(db_model) = related_db {
                            let related = <#entity_type as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                                .map_err(|e| #crate_path::db::DatabaseError::from(e))?;
                            entities.push((related, edge));
                        }
                    }

                    self.#field_name = entities;
                }
            } else {
                // If we can't extract tuple types, use the edge_entity_type string parameter
                let _edge_type_ident = syn::Ident::new(edge_entity_type, proc_macro2::Span::call_site());
                quote! {
                    // Load Vec<EdgeEntity> relations - fallback path
                    let query = format!("SELECT *, out.* as related_data FROM {} WHERE in = $parent ORDER BY id ASC", #relation_name);

                    let mut result = db.query(&query)
                        .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                        .await?;

                    // For now, initialize as empty with proper type annotation
                    // We need to default to an empty Vec but Rust can't infer the type
                    self.#field_name = Default::default();
                }
            }
        } else if is_option_type(field_type) {
            // Check if inner type is a tuple
            let inner_type = extract_inner_type(field_type).expect("Option should have inner type");
            if is_tuple_type(inner_type) {
                // Option<(Entity, EdgeEntity)> with edge entity
                // Extract tuple types directly from the Option's inner type
                if let Some((entity_type, edge_type)) = extract_tuple_types(inner_type) {
                    quote! {
                        // Load Option<(Entity, EdgeEntity)> with edge entity relation
                        let query = format!("SELECT *, out.* as related_data FROM {} WHERE in = $parent ORDER BY id ASC LIMIT 1", #relation_name);

                        let mut result = db.query(&query)
                            .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                            .await?;

                        // Extract the edge entity
                        let edge_records: Vec<serde_json::Value> = result.take(0)
                            ?;

                        if let Some(record) = edge_records.into_iter().next() {
                            // Extract the edge entity fields
                            let edge_obj = record.as_object()
                                .ok_or_else(|| #crate_path::db::DatabaseError::QueryFailed(
                                    ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                        "Edge record is not an object".into()
                                    ))
                                ))?;

                            // Get the related entity data
                            let related_data = edge_obj.get("related_data")
                                .ok_or_else(|| #crate_path::db::DatabaseError::QueryFailed(
                                    ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                        "No related_data field in edge query result".into()
                                    ))
                                ))?;

                            // Create edge entity from the record (minus related_data)
                            let mut edge_data = record.clone();
                            if let Some(obj) = edge_data.as_object_mut() {
                                obj.remove("related_data");
                            }

                            // Deserialize both entities
                            let edge_db: <#edge_type as #crate_path::db::entity::DbEntity>::DbModel =
                                serde_json::from_value(edge_data)
                                    .map_err(|e| #crate_path::db::DatabaseError::SerdeProblem(e))?;
                            let edge = <#edge_type as #crate_path::db::entity::DbEntity>::from_db_model(edge_db)
                                .map_err(|e| #crate_path::db::DatabaseError::from(e))?;

                            // Deserialize the related entity
                            let related_db: <#entity_type as #crate_path::db::entity::DbEntity>::DbModel =
                                serde_json::from_value(related_data.clone())
                                    .map_err(|e| #crate_path::db::DatabaseError::SerdeProblem(e))?;
                            let related = <#entity_type as #crate_path::db::entity::DbEntity>::from_db_model(related_db)
                                .map_err(|e| #crate_path::db::DatabaseError::from(e))?;

                            self.#field_name = Some((related, edge));
                        } else {
                            self.#field_name = None::<(#entity_type, #edge_type)>;
                        }
                    }
                } else {
                    panic!("Option edge entity field must contain tuple type");
                }
            } else {
                // Option<EdgeEntity> without tuple
                // This case shouldn't happen for edge entities - they should always be tuples
                quote! {
                    // TODO: Load Option<EdgeEntity> relation (not a tuple)
                    self.#field_name = None;
                }
            }
        } else {
            // Check if the field is a tuple type
            if is_tuple_type(field_type) {
                // Single (Entity, EdgeEntity) with edge entity
                // Extract tuple types directly from the field type
                if let Some((entity_type, edge_type)) = extract_tuple_types(field_type) {
                    quote! {
                        // Load single (Entity, EdgeEntity) with edge entity relation
                        let query = format!("SELECT *, out.* as related_data FROM {} WHERE in = $parent ORDER BY id ASC LIMIT 1", #relation_name);

                        let mut result = db.query(&query)
                            .bind(("parent", ::surrealdb::RecordId::from(self.id.clone())))
                            .await?;

                        // Extract the edge entity
                        let edge_records: Vec<serde_json::Value> = result.take(0)
                            ?;

                        let record = edge_records.into_iter().next()
                            .ok_or_else(|| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    format!("Required edge entity relation {} not found", stringify!(#field_name))
                                ))
                            ))?;

                        // Extract the edge entity fields
                        let edge_obj = record.as_object()
                            .ok_or_else(|| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    "Edge record is not an object".into()
                                ))
                            ))?;

                        // Get the related entity data
                        let related_data = edge_obj.get("related_data")
                            .ok_or_else(|| #crate_path::db::DatabaseError::QueryFailed(
                                ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                                    "No related_data field in edge query result".into()
                                ))
                            ))?;

                        // Create edge entity from the record (minus related_data)
                        let mut edge_data = record.clone();
                        if let Some(obj) = edge_data.as_object_mut() {
                            obj.remove("related_data");
                        }

                        // Deserialize both entities
                        let edge_db: <#edge_type as #crate_path::db::entity::DbEntity>::DbModel =
                            serde_json::from_value(edge_data)
                                .map_err(|e| #crate_path::db::DatabaseError::SerdeProblem(e))?;
                        let edge = <#edge_type as #crate_path::db::entity::DbEntity>::from_db_model(edge_db)
                            .map_err(|e| #crate_path::db::DatabaseError::from(e))?;

                        // Deserialize the related entity
                        let related_db: <#entity_type as #crate_path::db::entity::DbEntity>::DbModel =
                            serde_json::from_value(related_data.clone())
                                .map_err(|e| #crate_path::db::DatabaseError::SerdeProblem(e))?;
                        let related = <#entity_type as #crate_path::db::entity::DbEntity>::from_db_model(related_db)
                            .map_err(|e| #crate_path::db::DatabaseError::from(e))?;

                        self.#field_name = (related, edge);
                    }
                } else {
                    panic!("Edge entity field must be (Entity, EdgeEntity) but got: {:?}", quote! { #field_type }.to_string());
                }
            } else {
                // Single EdgeEntity without tuple
                quote! {
                    // TODO: Load single EdgeEntity relation (not a tuple)
                    self.#field_name = Default::default();
                }
            }
        }
    });

    // Generate statements to copy relation fields from self to stored
    let relation_copy_statements: Vec<_> = relation_fields
        .iter()
        .map(|(field_name, _, _)| {
            quote! {
                stored.#field_name = self.#field_name.clone();
            }
        })
        .collect();

    // Generate statements to copy edge entity fields from self to stored
    let edge_entity_copy_statements: Vec<_> = edge_entity_fields
        .iter()
        .map(|(field_name, _, _, _)| {
            quote! {
                stored.#field_name = self.#field_name.clone();
            }
        })
        .collect();

    // Generate different implementations for edge entities
    let store_with_relations_impl = if is_edge_entity {
        // Edge entities are created via RELATE, not directly stored
        quote! {
            /// Edge entities cannot be stored directly - use RELATE instead
            pub async fn store_with_relations<C: ::surrealdb::Connection>(
                &self,
                _db: &::surrealdb::Surreal<C>,
            ) -> std::result::Result<Self, #crate_path::db::DatabaseError> {
                Err(#crate_path::db::DatabaseError::QueryFailed(
                    ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                        "Edge entities must be created using RELATE, not stored directly".into()
                    ))
                ))
            }
        }
    } else {
        // Regular entity implementation
        quote! {
            /// Store entity to database with all relations
            pub async fn store_with_relations<C: ::surrealdb::Connection>(
                &self,
                db: &::surrealdb::Surreal<C>,
            ) -> std::result::Result<Self, #crate_path::db::DatabaseError> {
                // First upsert the entity
                let stored_db_model: Option<#db_model_name> = db
                    .upsert((<Self as #crate_path::db::entity::DbEntity>::table_name(), self.id.to_record_id()))
                    .content(<Self as #crate_path::db::entity::DbEntity>::to_db_model(self))
                    .await
                    ?;

                let stored_db_model = stored_db_model
                    .ok_or_else(|| #crate_path::db::DatabaseError::QueryFailed(
                        ::surrealdb::Error::Api(::surrealdb::error::Api::Query("Failed to upsert entity".into()))
                    ))?;

                let mut stored = <Self as #crate_path::db::entity::DbEntity>::from_db_model(stored_db_model)
                    .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                        ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert entity: {:?}", e)))
                    ))?;

                // Copy relation fields from original entity
                #(
                    #relation_copy_statements
                )*
                #(
                    #edge_entity_copy_statements
                )*

                // Then store all relations
                stored.store_relations(db).await?;

                Ok(stored)
            }
        }
    };

    let load_with_relations_impl = if is_edge_entity {
        // Edge entities are loaded differently
        quote! {
            /// Edge entities cannot be loaded directly - query the edge table instead
            pub async fn load_with_relations<C: ::surrealdb::Connection>(
                _db: &::surrealdb::Surreal<C>,
                _id: &#id_type,
            ) -> std::result::Result<Option<Self>, #crate_path::db::DatabaseError> {
                Err(#crate_path::db::DatabaseError::QueryFailed(
                    ::surrealdb::Error::Api(::surrealdb::error::Api::Query(
                        "Edge entities must be queried using the edge table, not loaded directly".into()
                    ))
                ))
            }
        }
    } else if entity_type == "message" {
        // Special case for Message entity which uses MessageId directly
        quote! {
            /// Load entity from database with all relations
            pub async fn load_with_relations<C: ::surrealdb::Connection>(
                db: &::surrealdb::Surreal<C>,
                id: &#crate_path::MessageId,
            ) -> std::result::Result<Option<Self>, #crate_path::db::DatabaseError> {
                // First load the entity - MessageId already has to_record_id() method
                let db_model: Option<#db_model_name> = db
                    .select((<Self as #crate_path::db::entity::DbEntity>::table_name(), id.to_record_id()))
                    .await?;

                if let Some(db_model) = db_model {
                    let mut entity = <Self as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                        .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert entity: {:?}", e)))
                        ))?;

                    // Then load all relations
                    entity.load_relations(db).await?;

                    Ok(Some(entity))
                } else {
                    Ok(None)
                }
            }
        }
    } else if entity_type == "atproto_identity" {
        // Special case for Message entity which uses MessageId directly
        quote! {
            /// Load entity from database with all relations
            pub async fn load_with_relations<C: ::surrealdb::Connection>(
                db: &::surrealdb::Surreal<C>,
                id: &#crate_path::Did,
            ) -> std::result::Result<Option<Self>, #crate_path::db::DatabaseError> {
                // First load the entity - MessageId already has to_record_id() method
                let db_model: Option<#db_model_name> = db
                    .select((<Self as #crate_path::db::entity::DbEntity>::table_name(), id.to_record_id()))
                    .await?;

                if let Some(db_model) = db_model {
                    let mut entity = <Self as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                        .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert entity: {:?}", e)))
                        ))?;

                    // Then load all relations
                    entity.load_relations(db).await?;

                    Ok(Some(entity))
                } else {
                    Ok(None)
                }
            }
        }
    } else {
        // Regular entity implementation
        quote! {
            /// Load entity from database with all relations
            pub async fn load_with_relations<C: ::surrealdb::Connection>(
                db: &::surrealdb::Surreal<C>,
                id: &#id_type,
            ) -> std::result::Result<Option<Self>, #crate_path::db::DatabaseError> {
                // First load the entity
                let db_model: Option<#db_model_name> = db
                    .select((<Self as #crate_path::db::entity::DbEntity>::table_name(), id.to_record_id()))
                    .await?;

                if let Some(db_model) = db_model {
                    let mut entity = <Self as #crate_path::db::entity::DbEntity>::from_db_model(db_model)
                        .map_err(|e| #crate_path::db::DatabaseError::QueryFailed(
                            ::surrealdb::Error::Api(::surrealdb::error::Api::Query(format!("Failed to convert entity: {:?}", e)))
                        ))?;

                    // Then load all relations
                    entity.load_relations(db).await?;

                    Ok(Some(entity))
                } else {
                    Ok(None)
                }
            }
        }
    };

    let id_method_impl = quote! {
        fn id(&self) -> &Self::Id  {
            &self.id
        }
    };

    let expanded = quote! {
        // Generate the storage model struct
        #[derive(Debug, Clone, ::serde::Serialize, ::serde::Deserialize)]
        pub struct #db_model_name {
            #(#storage_fields,)*
        }

        impl #name {
            /// Store all relation fields to the database
            pub async fn store_relations<C: ::surrealdb::Connection>(
                &self,
                db: &::surrealdb::Surreal<C>,
            ) -> ::std::result::Result<(), #crate_path::db::DatabaseError> {
                #(#store_relation_calls)*
                #(#store_edge_entity_calls)*
                Ok(())
            }

            /// Load all relation fields from the database
            pub async fn load_relations<C: ::surrealdb::Connection>(
                &mut self,
                db: &::surrealdb::Surreal<C>,
            ) -> ::std::result::Result<(), #crate_path::db::DatabaseError> {
                #(#load_relation_calls)*
                #(#load_edge_entity_calls)*
                Ok(())
            }


            /// Helper to extract table name from type string
            fn related_table_from_type(type_str: &str) -> &'static str {
                if type_str.contains("User") {
                    "user"
                } else if type_str.contains("Agent") {
                    "agent"
                } else if type_str.contains("Task") {
                    "task"
                } else if type_str.contains("Memory") {
                    "mem"
                } else if type_str.contains("Event") {
                    "event"
                } else {
                    panic!("unknown table name")
                }
            }

            /// Helper to extract table name from ID type string
            fn related_table_from_id_type(type_str: &str) -> &'static str {
                if type_str.contains("UserId") {
                    "user"
                } else if type_str.contains("AgentId") {
                    "agent"
                } else if type_str.contains("TaskId") {
                    "task"
                } else if type_str.contains("MemoryId") {
                    "mem"
                } else if type_str.contains("EventId") {
                    "event"
                } else {
                    panic!("unknown table name")
                }
            }

            #store_with_relations_impl

            #load_with_relations_impl
        }

        impl #crate_path::db::entity::DbEntity for #name {
            type DbModel = #db_model_name;
            type Domain = Self;
            type Id = #id_type;

            fn to_db_model(&self) -> Self::DbModel {
                #db_model_name {
                    #(#to_storage_conversions),*
                }
            }

            fn from_db_model(db_model: Self::DbModel) -> ::std::result::Result<Self::Domain, #crate_path::db::entity::EntityError> {
                Ok(Self {
                    #(#from_storage_conversions),*
                })
            }

            fn table_name() -> &'static str {
                #table_name
            }

            #id_method_impl

            fn schema() -> #crate_path::db::schema::TableDefinition {
                #helper_fn()
            }

            fn field_keys() -> Vec<String> {
                #field_keys_fn()
            }
        }

        // Generate schema helper function
        fn #helper_fn() -> #crate_path::db::schema::TableDefinition {
            let mut schema = format!(
                "DEFINE TABLE OVERWRITE {} SCHEMALESS;\n",
                #table_name
            );

            // Add field definitions
            let field_defs = vec![#(#field_definitions),*];
            for field_def in field_defs {
                schema.push_str(&field_def);
                schema.push_str(";\n");
            }

            #crate_path::db::schema::TableDefinition {
                name: #table_name.to_string(),
                schema,
                indexes: ::std::vec::Vec::new(),
            }
        }

        // Generate field keys helper function
        fn #field_keys_fn() -> ::std::vec::Vec<::std::string::String> {
            let mut keys = ::std::vec::Vec::new();
            #(
                keys.push(#storage_field_names);
            )*
            keys
        }
    };

    TokenStream::from(expanded)
}

fn determine_storage_type(
    _entity_type: &str,
    field_name: &Ident,
    field_type: &Type,
    field_opts: &FieldOpts,
) -> proc_macro2::TokenStream {
    // If a custom db_type is specified, use that
    if let Some(db_type) = &field_opts.db_type {
        // Special case: if db_type = "object", we want to store as serde_json::Value
        // (the field definition will use FLEXIBLE TYPE object)
        if db_type == "object" {
            return quote! { serde_json::Value };
        }
        let ty: Type = syn::parse_str(db_type).expect("Invalid db_type");
        return quote! { #ty };
    }

    let field_str = field_name.to_string();

    // Special handling for common fields
    match field_str.as_str() {
        "id" => {
            // Check if it's Option<RecordId> (edge entity case)
            let type_str = quote! { #field_type }.to_string();
            if type_str.contains("Option") && type_str.contains("RecordId") {
                // Edge entity with Option<RecordId>
                quote! { Option<::surrealdb::RecordId> }
            } else {
                // Regular entity - ID fields are stored as RecordId
                quote! { ::surrealdb::RecordId }
            }
        }
        "created_at" | "updated_at" | "scheduled_for" | "last_active" | "expires_at"
        | "last_used_at" => {
            // Check if it's wrapped in Option
            if is_option_type(field_type) {
                quote! { Option<::surrealdb::Datetime> }
            } else {
                quote! { ::surrealdb::Datetime }
            }
        }
        "due_date" | "completed_at" => {
            quote! { Option<::surrealdb::Datetime> }
        }
        "embedding" => quote! { Option<Vec<f32>> },
        _ => {
            // Check if this is an ID field (ends with _id)
            if is_id_type(field_type) {
                // ID fields are stored as RecordId
                if is_option_type(field_type) {
                    quote! { Option<::surrealdb::RecordId> }
                } else {
                    quote! { ::surrealdb::RecordId }
                }
            } else {
                // Check for special types that can be stored natively
                let type_str = quote! { #field_type }.to_string();
                if type_str.contains("serde_json") && type_str.contains("Value") {
                    // serde_json::Value can be stored natively as flexible field
                    quote! { #field_type }
                } else if type_str.contains("CompactString") {
                    // CompactString is stored as String
                    quote! { String }
                } else {
                    // Default: use the same type
                    quote! { #field_type }
                }
            }
        }
    }
}

fn generate_to_storage(
    field_name: &Ident,
    field_type: &Type,
    storage_type: &proc_macro2::TokenStream,
    needs_custom_conversion: bool,
) -> proc_macro2::TokenStream {
    let field_str = field_name.to_string();

    // Handle custom conversions for db_type
    if needs_custom_conversion {
        // Check common patterns - but skip for serde_json::Value
        let type_str = quote! { #field_type }.to_string();
        let storage_str = quote! { #storage_type }.to_string();

        if type_str.contains("serde_json") && type_str.contains("Value") {
            // serde_json::Value is stored natively, no conversion needed
            return quote! { #field_name: self.#field_name.clone() };
        } else if is_vec_to_string(field_type, storage_type) {
            return quote! {
                #field_name: self.#field_name.join(",")
            };
        } else if type_str.contains("CompactString") {
            // CompactString -> String conversion
            return quote! {
                #field_name: self.#field_name.to_string()
            };
        } else if storage_str.contains("serde_json") && storage_str.contains("Value") {
            // Converting to serde_json::Value for db_type = "object"
            return quote! {
                #field_name: serde_json::to_value(&self.#field_name)
                    .expect("Failed to serialize to JSON")
            };
        }
        // For other custom conversions, assume a to_storage method exists
        return quote! {
            #field_name: self.#field_name.to_storage()
        };
    }

    match field_str.as_str() {
        "id" => {
            // Check if it's Option<RecordId> (edge entity case)
            let type_str = quote! { #field_type }.to_string();
            if type_str.contains("Option") && type_str.contains("RecordId") {
                // Edge entity with Option<RecordId> - use as is
                quote! { #field_name: self.#field_name.clone() }
            } else if type_str.contains("MessageId") {
                // Special case for MessageId which doesn't implement From<MessageId> for RecordId
                quote! { #field_name: ::surrealdb::RecordId::from(self.#field_name.clone()) }
            } else if type_str.contains("Did") {
                // Special case for MessageId which doesn't implement From<MessageId> for RecordId
                quote! { #field_name: ::surrealdb::RecordId::from(self.#field_name.clone()) }
            } else {
                // Regular entity with custom ID type
                quote! { #field_name: ::surrealdb::RecordId::from(&self.#field_name.clone()) }
            }
        }
        "created_at" | "updated_at" | "scheduled_for" | "last_active" | "expires_at"
        | "last_used_at" => {
            if is_option_type(field_type) {
                quote! { #field_name: self.#field_name.map(::surrealdb::Datetime::from) }
            } else {
                quote! { #field_name: ::surrealdb::Datetime::from(self.#field_name) }
            }
        }
        "due_date" | "completed_at" => {
            quote! { #field_name: self.#field_name.map(::surrealdb::Datetime::from) }
        }
        _ => {
            // Check if this is an ID field (ends with _id)
            if is_id_type(field_type) {
                // Special handling for MessageId
                let type_str = quote! { #field_type }.to_string();
                if type_str.contains("MessageId") {
                    // MessageId needs clone() because it's not Copy
                    if is_option_type(field_type) {
                        quote! { #field_name: self.#field_name.clone().map(|id| ::surrealdb::RecordId::from(id)) }
                    } else {
                        quote! { #field_name: ::surrealdb::RecordId::from(self.#field_name.clone()) }
                    }
                } else {
                    // Regular ID types - always clone for both Copy and non-Copy types
                    if is_option_type(field_type) {
                        quote! { #field_name: self.#field_name.clone().map(|id| ::surrealdb::RecordId::from(id)) }
                    } else {
                        quote! { #field_name: ::surrealdb::RecordId::from(self.#field_name.clone()) }
                    }
                }
            } else {
                // Check if it's a CompactString
                let type_str = quote! { #field_type }.to_string();
                if type_str.contains("CompactString") {
                    quote! { #field_name: self.#field_name.to_string() }
                } else {
                    quote! { #field_name: self.#field_name.clone() }
                }
            }
        }
    }
}

fn generate_from_storage(
    field_name: &Ident,
    field_type: &Type,
    storage_type: &proc_macro2::TokenStream,
    crate_path: &syn::Path,
    needs_custom_conversion: bool,
    entity_type: &str,
    is_edge_entity: bool,
) -> proc_macro2::TokenStream {
    let field_str = field_name.to_string();

    // Handle custom conversions for db_type
    if needs_custom_conversion {
        // Check common patterns - but skip for serde_json::Value
        let type_str = quote! { #field_type }.to_string();
        let storage_str = quote! { #storage_type }.to_string();

        if type_str.contains("serde_json") && type_str.contains("Value") {
            // serde_json::Value is stored natively, no conversion needed
            return quote! { #field_name: db_model.#field_name };
        } else if is_vec_to_string(field_type, storage_type) {
            return quote! {
                #field_name: if db_model.#field_name.is_empty() {
                    Vec::new()
                } else {
                    db_model.#field_name.split(',')
                        .map(|s| s.trim().to_string())
                        .collect()
                }
            };
        } else if type_str.contains("CompactString") {
            // String -> CompactString conversion
            return quote! {
                #field_name: ::compact_str::CompactString::from(db_model.#field_name)
            };
        } else if storage_str.contains("serde_json") && storage_str.contains("Value") {
            // Converting from serde_json::Value for db_type = "object"
            return quote! {
                #field_name: serde_json::from_value(db_model.#field_name)
                    .map_err(|e| #crate_path::db::entity::EntityError::Serialization(e))?
            };
        }
        // For other custom conversions, assume a from_storage method exists
        return quote! {
            #field_name: <#field_type>::from_storage(db_model.#field_name)?
        };
    }

    match field_str.as_str() {
        "id" => {
            // Check if it's Option<RecordId> (edge entity case)
            let type_str = quote! { #field_type }.to_string();
            if type_str.contains("Option") && type_str.contains("RecordId") {
                // Edge entity with Option<RecordId>
                quote! { #field_name: db_model.#field_name }
            } else if entity_type == "message" {
                // Special case for MessageId which stores the full prefixed string
                quote! {
                    #field_name: #crate_path::MessageId(
                        #crate_path::db::strip_brackets(&db_model.#field_name.key().to_string()).to_string()
                    )
                }
            } else if entity_type == "atproto_identity" {
                // Special case for Did which stores the full prefixed string
                quote! {
                    #field_name: #crate_path::Did(::atrium_api::types::string::Did::new(
                        #crate_path::db::strip_brackets(&db_model.#field_name.key().to_string()).to_string()
                    ).unwrap())
                }
            } else {
                // Regular entity with custom ID type
                quote! {
                    #field_name: {
                        let id_str = db_model.#field_name.key().to_string();
                        let uuid_str = id_str.trim_start_matches('⟨').trim_end_matches('⟩');

                        <#field_type as #crate_path::id::IdType>::from_key(uuid_str).unwrap()
                    }
                }
            }
        }
        "created_at" | "updated_at" | "scheduled_for" | "last_active" | "expires_at"
        | "last_used_at" => {
            if is_option_type(field_type) {
                quote! { #field_name: db_model.#field_name.map(#crate_path::db::from_surreal_datetime) }
            } else {
                quote! { #field_name: #crate_path::db::from_surreal_datetime(db_model.#field_name) }
            }
        }
        "due_date" | "completed_at" => {
            quote! { #field_name: db_model.#field_name.map(#crate_path::db::from_surreal_datetime) }
        }
        _ => {
            // Check if this is an ID field (ends with _id)
            if is_id_type(field_type) {
                // Special handling for MessageId
                let type_str = quote! { #field_type }.to_string();
                if type_str.contains("MessageId") {
                    // MessageId stores the full string and uses from_record()
                    if is_edge_entity {
                        // Edge entities need special handling because SurrealDB may wrap the ID
                        if is_option_type(field_type) {
                            quote! {
                                #field_name: if let Some(record_id) = db_model.#field_name {
                                    let key = record_id.key().to_string();
                                    let cleaned = #crate_path::db::strip_brackets(&key);
                                    Some(#crate_path::MessageId(cleaned.to_string()))
                                } else {
                                    None
                                }
                            }
                        } else {
                            quote! {
                                #field_name: {
                                    let key = db_model.#field_name.key().to_string();
                                    let cleaned = #crate_path::db::strip_brackets(&key);
                                    #crate_path::MessageId(cleaned.to_string())
                                }
                            }
                        }
                    } else {
                        // Regular entities use from_record()
                        if is_option_type(field_type) {
                            quote! {
                                #field_name: if let Some(record_id) = db_model.#field_name {
                                    Some(#crate_path::MessageId::from_record(record_id))
                                } else {
                                    None
                                }
                            }
                        } else {
                            quote! {
                                #field_name: #crate_path::MessageId::from_record(db_model.#field_name)
                            }
                        }
                    }
                } else if type_str.contains("Did") {
                    // MessageId stores the full string and uses from_record()
                    if is_edge_entity {
                        // Edge entities need special handling because SurrealDB may wrap the ID
                        if is_option_type(field_type) {
                            quote! {
                                #field_name: if let Some(record_id) = db_model.#field_name {
                                    let key = record_id.key().to_string();
                                    let cleaned = #crate_path::db::strip_brackets(&key);
                                    Some(#crate_path::Did(cleaned.to_string()))
                                } else {
                                    None
                                }
                            }
                        } else {
                            quote! {
                                #field_name: {
                                    let key = db_model.#field_name.key().to_string();
                                    let cleaned = #crate_path::db::strip_brackets(&key);
                                    #crate_path::Did(cleaned.to_string())
                                }
                            }
                        }
                    } else {
                        // Regular entities use from_record()
                        if is_option_type(field_type) {
                            quote! {
                                #field_name: if let Some(record_id) = db_model.#field_name {
                                    Some(#crate_path::Did::from_record(record_id))
                                } else {
                                    None
                                }
                            }
                        } else {
                            quote! {
                                #field_name: #crate_path::Did::from_record(db_model.#field_name)
                            }
                        }
                    }
                } else {
                    // Regular ID types use from_uuid()
                    if is_option_type(field_type) {
                        // Option<ID> case
                        let inner_type =
                            extract_inner_type(field_type).expect("Option should have inner type");
                        quote! {
                            #field_name: if let Some(record_id) = db_model.#field_name {
                                let id_str = record_id.key().to_string();
                                let uuid_str = id_str.trim_start_matches('⟨').trim_end_matches('⟩').trim();
                                let uuid = ::uuid::Uuid::parse_str(&uuid_str)
                                    .map_err(|e| #crate_path::db::entity::EntityError::InvalidId(
                                        #crate_path::id::IdError::InvalidUuid(e)
                                    ))?;
                                Some(#inner_type::from_uuid(uuid))
                            } else {
                                None
                            }
                        }
                    } else {
                        // Regular ID case
                        quote! {
                            #field_name: {
                                let id_str = db_model.#field_name.key().to_string();
                                let uuid_str = id_str.trim_start_matches('⟨').trim_end_matches('⟩').trim();
                                let uuid = ::uuid::Uuid::parse_str(&uuid_str)
                                    .map_err(|e| #crate_path::db::entity::EntityError::InvalidId(
                                        #crate_path::id::IdError::InvalidUuid(e)
                                    ))?;
                                #field_type::from_uuid(uuid)
                            }
                        }
                    }
                }
            } else {
                // Check if it's a CompactString
                let type_str = quote! { #field_type }.to_string();
                if type_str.contains("CompactString") {
                    quote! { #field_name: ::compact_str::CompactString::from(db_model.#field_name) }
                } else {
                    quote! { #field_name: db_model.#field_name }
                }
            }
        }
    }
}

fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first() {
            return segment.ident == "Option";
        }
    }
    false
}

fn extract_inner_type(ty: &Type) -> Option<&Type> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first() {
            if segment.ident == "Vec" || segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_type)) = args.args.first() {
                        return Some(inner_type);
                    }
                }
            }
        }
    }
    None
}

fn matches_type(type1: &proc_macro2::TokenStream, type2: &Type) -> bool {
    // This is a simplified check - in reality we'd need more sophisticated type comparison
    let type1_str = type1.to_string().replace(" ", "");
    let type2_str = quote! { #type2 }.to_string().replace(" ", "");
    type1_str == type2_str
}

fn is_vec_to_string(field_type: &Type, storage_type: &proc_macro2::TokenStream) -> bool {
    let storage_str = storage_type.to_string();

    // Check if it's Vec<String> -> String conversion
    if let Type::Path(type_path) = field_type {
        if let Some(segment) = type_path.path.segments.first() {
            if segment.ident == "Vec" {
                // Check if inner type is String
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_type)) = args.args.first() {
                        let inner_str = quote! { #inner_type }.to_string();
                        return inner_str == "String" && storage_str == "String";
                    }
                }
            }
        }
    }
    false
}

fn is_vec_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.first() {
            return segment.ident == "Vec";
        }
    }
    false
}

fn is_tuple_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(_))
}

/// Extract both types from a tuple type (A, B)
fn extract_tuple_types(ty: &Type) -> Option<(&Type, &Type)> {
    match ty {
        Type::Tuple(tuple) => {
            if tuple.elems.len() == 2 {
                let first = tuple.elems.first()?;
                let second = tuple.elems.iter().nth(1)?;
                Some((first, second))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extract tuple types from Vec<(A, B)> or Option<(A, B)>
fn extract_tuple_types_from_container(ty: &Type) -> Option<(&Type, &Type)> {
    if let Type::Path(path) = ty {
        if let Some(segment) = path.path.segments.last() {
            if segment.ident == "Vec" || segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                        return extract_tuple_types(inner);
                    }
                }
            }
        }
    }
    None
}

fn generate_field_definition(
    field_name: &Ident,
    storage_type: &proc_macro2::TokenStream,
    table_name: &str,
    field_opts: &FieldOpts,
) -> String {
    let field_str = field_name.to_string();

    // Special case: if db_type = "object", use FLEXIBLE TYPE object
    if let Some(db_type) = &field_opts.db_type {
        if db_type == "object" {
            return format!("DEFINE FIELD {field_str} ON TABLE {table_name} FLEXIBLE TYPE object");
        } else if db_type == "optional_object" {
            return format!(
                "DEFINE FIELD {} ON TABLE {} FLEXIBLE TYPE option<object>",
                field_str.strip_suffix("<option>").unwrap_or(&field_str),
                table_name
            );
        }
    }

    let type_str = storage_type.to_string();
    // Remove spaces from type string for matching
    let normalized_type = type_str.replace(" ", "");

    // Map storage types to SurrealDB field types
    let surreal_type = match normalized_type.as_str() {
        "::surrealdb::RecordId" => "TYPE record",
        "::surrealdb::Datetime" => "TYPE datetime",
        "Option<::surrealdb::Datetime>" => "TYPE option<datetime>",
        "DateTime<Utc>" => "TYPE datetime",
        "Option<DateTime<Utc>>" => "TYPE option<datetime>",
        "String" => "TYPE string",
        "Option<String>" => "TYPE option<string>",
        "bool" => "TYPE bool",
        "Option<bool>" => "TYPE option<bool>",
        "i32" | "i64" | "u32" | "u64" | "usize" => "TYPE int",
        "Option<i32>" | "Option<i64>" | "Option<u32>" | "Option<u64>" | "Option<usize>" => {
            "TYPE option<int>"
        }
        "f32" | "f64" => "TYPE float",
        "Option<f32>" | "Option<f64>" => "TYPE option<float>",
        "Vec<f32>" | "Option<Vec<f32>>" => "TYPE option<array<float>>",
        "Vec<String>" => "TYPE array<string>",
        "CompactString" => "TYPE string",
        _ => {
            // Check for special types
            if normalized_type.contains("serde_json") && normalized_type.contains("Value") {
                "FLEXIBLE TYPE object"
            } else if normalized_type.contains("HashMap")
                && normalized_type.contains("String")
                && normalized_type.contains("serde_json")
            {
                // HashMap<String, serde_json::Value> or similar
                "FLEXIBLE TYPE object"
            } else if normalized_type.contains("CompactString") {
                "TYPE string"
            } else if normalized_type.contains("Id") || normalized_type.contains("RecordId") {
                // ID types are records
                if normalized_type.starts_with("Option<") {
                    "TYPE option<record>"
                } else {
                    "TYPE record"
                }
            } else if normalized_type.starts_with("Option<") {
                // Check what's inside the Option
                if normalized_type.contains("Vec<") {
                    "TYPE option<array>"
                } else {
                    // For other Option types, use string as a safe default
                    "TYPE option<string>"
                }
            } else if normalized_type.contains("Vec<") {
                // Vec types that aren't caught above
                "TYPE array"
            } else {
                // For enums and other types, use string
                "TYPE string"
            }
        }
    };

    format!("DEFINE FIELD {field_str} ON TABLE {table_name} {surreal_type}")
}

fn is_id_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident_str = segment.ident.to_string();
            return ident_str.ends_with("Id") && !ident_str.ends_with("RecordId");
        }
    }
    false
}
