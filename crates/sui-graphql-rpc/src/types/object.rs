// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use async_graphql::{connection::Connection, *};
use diesel::{
    debug_query, CombineDsl, ExpressionMethods, NullableExpressionMethods, OptionalExtension,
    QueryDsl, RunQueryDsl,
};
use move_core_types::annotated_value::{MoveStruct, MoveTypeLayout};
use move_core_types::language_storage::StructTag;
use sui_indexer::models_v2::objects::{
    StoredDeletedHistoryObject, StoredHistoryObject, StoredObject,
};
use sui_indexer::schema_v2::{checkpoints, objects, objects_history, objects_snapshot};
use sui_indexer::types_v2::ObjectStatus as NativeObjectStatus;
use sui_json_rpc::name_service::NameServiceConfig;
use sui_package_resolver::Resolver;
use sui_types::dynamic_field::DynamicFieldType;
use sui_types::TypeTag;

use super::big_int::BigInt;
use super::display::{get_rendered_fields, DisplayEntry};
use super::dynamic_field::{DynamicField, DynamicFieldName};
use super::move_object::MoveObject;
use super::move_package::MovePackage;
use super::suins_registration::SuinsRegistration;
use super::{
    balance::Balance, coin::Coin, owner::Owner, stake::StakedSui, sui_address::SuiAddress,
    transaction_block::TransactionBlock,
};
use crate::context_data::db_data_provider::PgManager;
use crate::context_data::package_cache::PackageCache;
use crate::data::pg::coalesce;
use crate::data::{Db, QueryExecutor};
use crate::error::Error;
use crate::types::base64::Base64;
use sui_types::object::{
    MoveObject as NativeMoveObject, Object as NativeObject, Owner as NativeOwner,
};

#[derive(Clone, Debug)]
pub(crate) struct Object {
    pub address: SuiAddress,
    pub kind: ObjectKind,
}

#[derive(Clone, Debug)]
pub(crate) enum ObjectKind {
    /// An object loaded from serialized data, such as the contents of a transaction.
    NotIndexed(NativeObject),
    /// An object fetched from the live objects table.
    Live(NativeObject, StoredObject),
    /// An object fetched from the snapshot or historical objects table.
    Historical(NativeObject, StoredHistoryObject),
    /// The object is wrapped or deleted and only partial information can be loaded from the
    /// indexer.
    WrappedOrDeleted(StoredDeletedHistoryObject),
    /// The requested object falls outside of the consistent read range supported by the indexer.
    /// The requested object may or may not actually exist on-chain, but the data is not yet or no
    /// longer indexed.
    OutsideAvailableRange,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
#[graphql(name = "ObjectKind")]
pub enum GraphQLObjectKind {
    NotIndexed,
    Live,
    Historical,
    WrappedOrDeleted,
    OutsideAvailableRange,
}

#[derive(InputObject, Default, Clone)]
pub(crate) struct ObjectFilter {
    /// This field is used to specify the type of objects that should be included in the query
    /// results.
    ///
    /// Objects can be filtered by their type's package, package::module, or their fully qualified
    /// type name.
    ///
    /// Generic types can be queried by either the generic type name, e.g. `0x2::coin::Coin`, or by
    /// the full type name, such as `0x2::coin::Coin<0x2::sui::SUI>`.
    pub type_: Option<String>,

    /// Filter for live objects by their current owners.
    pub owner: Option<SuiAddress>,

    /// Filter for live objects by their IDs.
    pub object_ids: Option<Vec<SuiAddress>>,

    /// Filter for live or potentially historical objects by their ID and version.
    pub object_keys: Option<Vec<ObjectKey>>,
}

#[derive(InputObject, Clone)]
pub(crate) struct ObjectKey {
    object_id: SuiAddress,
    version: u64,
}

/// The object's owner type: Immutable, Shared, Parent, or Address.
#[derive(Union, Clone)]
pub enum ObjectOwner {
    Immutable(Immutable),
    Shared(Shared),
    Parent(Parent),
    Address(AddressOwner),
}

/// An immutable object is an object that can't be mutated, transferred, or deleted.
/// Immutable objects have no owner, so anyone can use them.
#[derive(SimpleObject, Clone)]
pub struct Immutable {
    #[graphql(name = "_")]
    dummy: Option<bool>,
}

/// A shared object is an object that is shared using the 0x2::transfer::share_object function
/// and is accessible to everyone.
/// Unlike owned objects, once an object is shared, it stays mutable and can be accessed by anyone,
/// unless it is made immutable. An example of immutable shared objects are all published packages
/// and modules on Sui.
#[derive(SimpleObject, Clone)]
pub struct Shared {
    initial_shared_version: u64,
}

/// The parent of this object
#[derive(SimpleObject, Clone)]
pub struct Parent {
    parent: Option<Object>,
}

/// An address-owned object is owned by a specific 32-byte address that is
/// either an account address (derived from a particular signature scheme) or
/// an object ID. An address-owned object is accessible only to its owner and no others.
#[derive(SimpleObject, Clone)]
pub struct AddressOwner {
    owner: Option<Owner>,
}

#[Object]
impl Object {
    async fn version(&self) -> Option<u64> {
        self.version_impl()
    }

    /// The current status of the object as read from the off-chain store. The possible states are:
    /// - Live: the object is currently live and is not deleted or wrapped.
    /// - NotIndexed: the object is loaded from serialized data, such as the contents of a
    ///   transaction.
    /// - WrappedOrDeleted: The object is deleted or wrapped and only partial information can be
    ///   loaded from the indexer.
    /// - OutsideAvailableRange: The requested object falls outside of the consistent read range
    /// supported by the indexer. The requested object may or may not actually exist on-chain, but
    /// the data is not yet or no longer indexed.
    async fn status(&self) -> GraphQLObjectKind {
        GraphQLObjectKind::from(&self.kind)
    }

    /// 32-byte hash that identifies the object's current contents, encoded as a Base58 string.
    async fn digest(&self) -> Option<String> {
        self.native_impl()
            .map(|native| native.digest().base58_encode())
    }

    /// The amount of SUI we would rebate if this object gets deleted or mutated.
    /// This number is recalculated based on the present storage gas price.
    async fn storage_rebate(&self) -> Option<BigInt> {
        self.native_impl()
            .map(|native| BigInt::from(native.storage_rebate))
    }

    /// The set of named templates defined on-chain for the type of this object,
    /// to be handled off-chain. The server substitutes data from the object
    /// into these templates to generate a display string per template.
    async fn display(&self, ctx: &Context<'_>) -> Result<Option<Vec<DisplayEntry>>> {
        let Some(native) = self.native_impl() else {
            return Ok(None);
        };

        let resolver: &Resolver<PackageCache> = ctx
            .data()
            .map_err(|_| Error::Internal("Unable to fetch Package Cache.".to_string()))
            .extend()?;
        let move_object = native
            .data
            .try_as_move()
            .ok_or_else(|| Error::Internal("Failed to convert object into MoveObject".to_string()))
            .extend()?;

        let (struct_tag, move_struct) = deserialize_move_struct(move_object, resolver)
            .await
            .extend()?;

        let stored_display = ctx
            .data_unchecked::<PgManager>()
            .fetch_display_object_by_type(&struct_tag)
            .await
            .extend()?;

        let Some(stored_display) = stored_display else {
            return Ok(None);
        };

        let event = stored_display
            .to_display_update_event()
            .map_err(|e| Error::Internal(e.to_string()))
            .extend()?;

        Ok(Some(
            get_rendered_fields(event.fields, &move_struct).extend()?,
        ))
    }

    /// The Base64 encoded bcs serialization of the object's content.
    async fn bcs(&self) -> Result<Option<Base64>> {
        let Some(native) = self.native_impl() else {
            return Ok(None);
        };

        let bytes = bcs::to_bytes(native)
            .map_err(|e| {
                Error::Internal(format!(
                    "Failed to serialize object at {}: {e}",
                    self.address,
                ))
            })
            .extend()?;

        Ok(Some(Base64::from(&bytes)))
    }

    /// The transaction block that created this version of the object.
    async fn previous_transaction_block(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Option<TransactionBlock>> {
        let Some(native) = self.native_impl() else {
            return Ok(None);
        };

        let digest = native.previous_transaction;
        ctx.data_unchecked::<PgManager>()
            .fetch_tx(&digest.into())
            .await
            .extend()
    }

    /// The owner type of this Object.
    /// The owner can be one of the following types: Immutable, Shared, Parent, Address
    /// Immutable and Shared Objects do not have owners.
    async fn owner(&self, ctx: &Context<'_>) -> Option<ObjectOwner> {
        use NativeOwner as O;

        let Some(native) = self.native_impl() else {
            return None;
        };

        match native.owner {
            O::AddressOwner(address) => {
                let address = SuiAddress::from(address);
                Some(ObjectOwner::Address(AddressOwner {
                    owner: Some(Owner { address }),
                }))
            }
            O::Immutable => Some(ObjectOwner::Immutable(Immutable { dummy: None })),
            O::ObjectOwner(address) => {
                let parent = Object::query(ctx.data_unchecked(), address.into(), None, None)
                    .await
                    .ok()
                    .flatten();

                return Some(ObjectOwner::Parent(Parent { parent }));
            }
            O::Shared {
                initial_shared_version,
            } => Some(ObjectOwner::Shared(Shared {
                initial_shared_version: initial_shared_version.value(),
            })),
        }
    }

    /// Attempts to convert the object into a MoveObject
    async fn as_move_object(&self) -> Option<MoveObject> {
        MoveObject::try_from(self).ok()
    }

    /// Attempts to convert the object into a MovePackage
    async fn as_move_package(&self) -> Option<MovePackage> {
        MovePackage::try_from(self).ok()
    }

    // =========== Owner interface methods =============

    /// The address of the object, named as such to avoid conflict with the address type.
    pub async fn address(&self) -> SuiAddress {
        self.address
    }

    /// The objects owned by this object
    pub async fn object_connection(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
        filter: Option<ObjectFilter>,
    ) -> Result<Option<Connection<String, Object>>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_owned_objs(first, after, last, before, filter, self.address)
            .await
            .extend()
    }

    /// The balance of coin objects of a particular coin type owned by the object.
    pub async fn balance(
        &self,
        ctx: &Context<'_>,
        type_: Option<String>,
    ) -> Result<Option<Balance>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_balance(self.address, type_)
            .await
            .extend()
    }

    /// The balances of all coin types owned by the object. Coins of the same type are grouped together into one Balance.
    pub async fn balance_connection(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
    ) -> Result<Option<Connection<String, Balance>>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_balances(self.address, first, after, last, before)
            .await
            .extend()
    }

    /// The coin objects for the given address.
    ///
    /// The type field is a string of the inner type of the coin by which to filter
    /// (e.g. `0x2::sui::SUI`). If no type is provided, it will default to `0x2::sui::SUI`.
    pub async fn coin_connection(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
        type_: Option<String>,
    ) -> Result<Option<Connection<String, Coin>>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_coins(Some(self.address), type_, first, after, last, before)
            .await
            .extend()
    }

    /// The `0x3::staking_pool::StakedSui` objects owned by the given object.
    pub async fn staked_sui_connection(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
    ) -> Result<Option<Connection<String, StakedSui>>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_staked_sui(self.address, first, after, last, before)
            .await
            .extend()
    }

    /// The domain that a user address has explicitly configured as their default domain
    pub async fn default_name_service_name(&self, ctx: &Context<'_>) -> Result<Option<String>> {
        ctx.data_unchecked::<PgManager>()
            .default_name_service_name(ctx.data_unchecked::<NameServiceConfig>(), self.address)
            .await
            .extend()
    }

    /// The SuinsRegistration NFTs owned by the given object. These grant the owner
    /// the capability to manage the associated domain.
    pub async fn suins_registrations(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
    ) -> Result<Option<Connection<String, SuinsRegistration>>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_suins_registrations(
                first,
                after,
                last,
                before,
                ctx.data_unchecked::<NameServiceConfig>(),
                self.address,
            )
            .await
            .extend()
    }

    /// Access a dynamic field on an object using its name.
    /// Names are arbitrary Move values whose type have `copy`, `drop`, and `store`, and are specified
    /// using their type, and their BCS contents, Base64 encoded.
    /// Dynamic fields on wrapped objects can be accessed by using the same API under the Owner type.
    pub async fn dynamic_field(
        &self,
        ctx: &Context<'_>,
        name: DynamicFieldName,
    ) -> Result<Option<DynamicField>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_dynamic_field(self.address, name, DynamicFieldType::DynamicField)
            .await
            .extend()
    }

    /// Access a dynamic object field on an object using its name.
    /// Names are arbitrary Move values whose type have `copy`, `drop`, and `store`, and are specified
    /// using their type, and their BCS contents, Base64 encoded.
    /// The value of a dynamic object field can also be accessed off-chain directly via its address (e.g. using `Query.object`).
    /// Dynamic fields on wrapped objects can be accessed by using the same API under the Owner type.
    pub async fn dynamic_object_field(
        &self,
        ctx: &Context<'_>,
        name: DynamicFieldName,
    ) -> Result<Option<DynamicField>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_dynamic_field(self.address, name, DynamicFieldType::DynamicObject)
            .await
            .extend()
    }

    /// The dynamic fields on an object.
    /// Dynamic fields on wrapped objects can be accessed by using the same API under the Owner type.
    pub async fn dynamic_field_connection(
        &self,
        ctx: &Context<'_>,
        first: Option<u64>,
        after: Option<String>,
        last: Option<u64>,
        before: Option<String>,
    ) -> Result<Option<Connection<String, DynamicField>>> {
        ctx.data_unchecked::<PgManager>()
            .fetch_dynamic_fields(first, after, last, before, self.address)
            .await
            .extend()
    }
}

impl Object {
    /// Construct a GraphQL object from a native object, without its stored (indexed) counterpart.
    pub(crate) fn from_native(address: SuiAddress, native: NativeObject) -> Object {
        Object {
            address,
            kind: ObjectKind::NotIndexed(native),
        }
    }

    pub(crate) fn native_impl(&self) -> Option<&NativeObject> {
        match &self.kind {
            ObjectKind::Live(native, _)
            | ObjectKind::NotIndexed(native)
            | ObjectKind::Historical(native, _) => Some(native),
            ObjectKind::WrappedOrDeleted(_) | ObjectKind::OutsideAvailableRange => None,
        }
    }

    pub(crate) fn version_impl(&self) -> Option<u64> {
        match &self.kind {
            ObjectKind::Live(native, _)
            | ObjectKind::NotIndexed(native)
            | ObjectKind::Historical(native, _) => Some(native.version().value()),
            ObjectKind::WrappedOrDeleted(stored) => Some(stored.object_version as u64),
            ObjectKind::OutsideAvailableRange => None,
        }
    }

    async fn live_object_query(db: &Db, address: &SuiAddress) -> Result<Option<Self>, Error> {
        let vec_address = address.into_vec();
        use objects::dsl as objects;

        let stored_obj: Option<StoredObject> = db
            .optional(move || {
                objects::objects
                    .filter(objects::object_id.eq(vec_address.clone()))
                    .limit(1)
                    .into_boxed()
            })
            .await?;

        stored_obj.map(Self::try_from).transpose()
    }

    async fn historical_object_query(
        db: &Db,
        address: &SuiAddress,
        version: Option<i64>,
        checkpoint_sequence_number: Option<i64>,
    ) -> Result<Option<Self>, Error> {
        let vec_address = address.into_vec();

        use checkpoints::dsl as checkpoints;
        use objects_history::dsl as history;
        use objects_snapshot::dsl as snapshot;

        let results: Option<Vec<StoredHistoryObject>> = db
            .inner
            .spawn_blocking(move |this| {
                this.run_consistent_query(|conn| {
                    // If an object was created or mutated in a checkpoint outside the current
                    // available range, and never touched again, it will not show up in the
                    // objects_history table. Thus, we always need to check the objects_snapshot
                    // table as well.
                    let mut snapshot_query = snapshot::objects_snapshot
                        .filter(snapshot::object_id.eq(vec_address.clone()))
                        .into_boxed();

                    let mut historical_query = history::objects_history
                        .filter(history::object_id.eq(vec_address.clone()))
                        .order_by(history::object_version.desc())
                        .limit(1)
                        .into_boxed();

                    if let Some(version) = version {
                        snapshot_query =
                            snapshot_query.filter(snapshot::object_version.eq(version));
                        historical_query =
                            historical_query.filter(history::object_version.eq(version));
                    }

                    let left = snapshot::objects_snapshot
                        .select(snapshot::checkpoint_sequence_number)
                        .order(snapshot::checkpoint_sequence_number.desc())
                        .limit(1);

                    if let Some(checkpoint_sequence_number) = checkpoint_sequence_number {
                        // We could make a validation check here that the provided
                        // checkpoint_sequence_number falls between the available range. However,
                        // this would incur another db roundtrip. Additionally, if
                        // checkpoint_sequence_number < left, the db would return 0 rows, so the
                        // check is unncessary as we need to make a roundtrip regardless.
                        historical_query = historical_query.filter(
                            history::checkpoint_sequence_number
                                .nullable()
                                .between(left.single_value(), checkpoint_sequence_number),
                        );
                    } else {
                        let right = checkpoints::checkpoints
                            .select(checkpoints::sequence_number)
                            .order(checkpoints::sequence_number.desc())
                            .limit(1);

                        historical_query =
                            historical_query.filter(history::checkpoint_sequence_number.between(
                                coalesce(left.single_value(), 0),
                                coalesce(right.single_value(), 0),
                            ));
                    }

                    let final_query = snapshot_query.union(historical_query);

                    let debug = debug_query(&final_query);

                    println!("Query: {:?}", debug);
                    println!("Query: {}", debug.to_string());

                    final_query.load(conn).optional()
                })
            })
            .await?;

        // If the object existed at some point, it should be found at least from snapshots.
        // Therefore, if both results are None, the object does not exist.
        let Some(stored_objs) = results else {
            return Ok(None);
        };
        println!("Stored objects: {:?}", stored_objs);

        match stored_objs
            .iter()
            // First, filter the objects based on the version and checkpoint_sequence_number if provided
            .filter(|stored_obj| {
                version.map_or(true, |ver| stored_obj.object_version == ver)
                    && checkpoint_sequence_number.map_or(true, |checkpoint| {
                        stored_obj.checkpoint_sequence_number <= checkpoint
                    })
            })
            // Then, find the object with the largest checkpoint_sequence_number among those
            // filtered - it should still be within the available range as the db query was bounded
            .max_by_key(|stored_obj| stored_obj.checkpoint_sequence_number)
        {
            Some(stored_obj) => {
                Ok(Some(Self::try_from(stored_obj.clone()).map_err(|e| {
                    Error::Internal(format!("Failed to convert object: {e}"))
                })?))
            }
            None => Ok(Some(Object {
                address: address.clone(),
                kind: ObjectKind::OutsideAvailableRange,
            })),
        }
    }

    pub(crate) async fn query(
        db: &Db,
        address: SuiAddress,
        version: Option<u64>,
        checkpoint_sequence_number: Option<u64>,
    ) -> Result<Option<Self>, Error> {
        let version = version.map(|v| v as i64);
        let checkpoint_sequence_number = checkpoint_sequence_number.map(|v| v as i64);

        if version.is_none() && checkpoint_sequence_number.is_none() {
            return Object::live_object_query(db, &address)
                .await
                .map_err(|e| Error::Internal(format!("Failed to fetch object: {e}")));
        } else {
            return Object::historical_object_query(
                db,
                &address,
                version,
                checkpoint_sequence_number,
            )
            .await
            .map_err(|e| Error::Internal(format!("Failed to fetch object: {e}")));
        }
    }
}

impl TryFrom<StoredObject> for Object {
    type Error = Error;

    fn try_from(stored_object: StoredObject) -> Result<Self, Error> {
        let address = addr(&stored_object.object_id)?;
        let native_object = bcs::from_bytes(&stored_object.serialized_object)
            .map_err(|_| Error::Internal(format!("Failed to deserialize object {address}")))?;

        Ok(Self {
            address,
            kind: ObjectKind::Live(native_object, stored_object),
        })
    }
}

impl TryFrom<StoredHistoryObject> for Object {
    type Error = Error;

    fn try_from(history_object: StoredHistoryObject) -> Result<Self, Error> {
        let address = addr(&history_object.object_id)?;

        let object_status =
            NativeObjectStatus::try_from(history_object.object_status).map_err(|_| {
                Error::Internal(format!(
                    "Unknown object status {} for object {} at version {}",
                    history_object.object_status, address, history_object.object_version
                ))
            })?;

        match object_status {
            NativeObjectStatus::Active => {
                let Some(serialized_object) = &history_object.serialized_object else {
                    return Err(Error::Internal(format!(
                        "Live object {} at version {} cannot have missing serialized_object field",
                        address, history_object.object_version
                    )));
                };

                let native_object = bcs::from_bytes(serialized_object).map_err(|_| {
                    Error::Internal(format!("Failed to deserialize object {address}"))
                })?;

                Ok(Self {
                    address,
                    kind: ObjectKind::Historical(native_object, history_object),
                })
            }
            NativeObjectStatus::WrappedOrDeleted => Ok(Self {
                address,
                kind: ObjectKind::WrappedOrDeleted(StoredDeletedHistoryObject {
                    object_id: history_object.object_id,
                    object_version: history_object.object_version,
                    object_status: history_object.object_status,
                    checkpoint_sequence_number: history_object.checkpoint_sequence_number,
                }),
            }),
        }
    }
}

impl From<&ObjectKind> for GraphQLObjectKind {
    fn from(kind: &ObjectKind) -> Self {
        match kind {
            ObjectKind::NotIndexed(_) => GraphQLObjectKind::NotIndexed,
            ObjectKind::Live(_, _) => GraphQLObjectKind::Live,
            ObjectKind::Historical(_, _) => GraphQLObjectKind::Historical,
            ObjectKind::WrappedOrDeleted(_) => GraphQLObjectKind::WrappedOrDeleted,
            ObjectKind::OutsideAvailableRange => GraphQLObjectKind::OutsideAvailableRange,
        }
    }
}

/// Parse a `SuiAddress` from its stored representation.  Failure is an internal error: the
/// database should never contain a malformed address (containing the wrong number of bytes).
fn addr(bytes: impl AsRef<[u8]>) -> Result<SuiAddress, Error> {
    SuiAddress::from_bytes(bytes.as_ref()).map_err(|e| {
        let bytes = bytes.as_ref().to_vec();
        Error::Internal(format!("Error deserializing address: {bytes:?}: {e}"))
    })
}

pub(crate) async fn deserialize_move_struct(
    move_object: &NativeMoveObject,
    resolver: &Resolver<PackageCache>,
) -> Result<(StructTag, MoveStruct), Error> {
    let struct_tag = StructTag::from(move_object.type_().clone());
    let contents = move_object.contents();
    let move_type_layout = resolver
        .type_layout(TypeTag::from(struct_tag.clone()))
        .await
        .map_err(|e| {
            Error::Internal(format!(
                "Error fetching layout for type {}: {e}",
                struct_tag.to_canonical_string(/* with_prefix */ true)
            ))
        })?;

    let MoveTypeLayout::Struct(layout) = move_type_layout else {
        return Err(Error::Internal("Object is not a move struct".to_string()));
    };

    let move_struct = MoveStruct::simple_deserialize(contents, &layout).map_err(|e| {
        Error::Internal(format!(
            "Error deserializing move struct for type {}: {e}",
            struct_tag.to_canonical_string(/* with_prefix */ true)
        ))
    })?;

    Ok((struct_tag, move_struct))
}
