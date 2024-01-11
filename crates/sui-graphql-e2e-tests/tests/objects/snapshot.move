// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

// Objects can continue to be found on the live objects table until they are WrappedOrDeleted. From
// there, the object can be fetched on the objects_history table, until it gets snapshotted into
// objects_snapshot table. This test checks that we also fetch from objects_snapshot, by creating an
// object at checkpoint 1, wrapping it at checkpoint 2, and progressing enough checkpoints that the
// wrapped object gets written to objects_snapshot. At this point, the object at the initial
// version, 3, should no longer be fetchable.

//# init --addresses Test=0x0 --accounts A --simulator --env-vars OBJECTS_SNAPSHOT_MIN_CHECKPOINT_LAG=0 OBJECTS_SNAPSHOT_MAX_CHECKPOINT_LAG=2

//# publish
module Test::M1 {
    use sui::object::{Self, UID};
    use sui::tx_context::{Self, TxContext};
    use sui::transfer;

    struct Object has key, store {
        id: UID,
        value: u64,
    }

    struct Wrapper has key {
        id: UID,
        o: Object
    }

    public entry fun create(value: u64, recipient: address, ctx: &mut TxContext) {
        transfer::public_transfer(
            Object { id: object::new(ctx), value },
            recipient
        )
    }

    public entry fun update(o1: &mut Object, value: u64,) {
        o1.value = value;
    }

    public entry fun wrap(o: Object, ctx: &mut TxContext) {
        transfer::transfer(Wrapper { id: object::new(ctx), o }, tx_context::sender(ctx))
    }

    public entry fun unwrap(w: Wrapper, ctx: &mut TxContext) {
        let Wrapper { id, o } = w;
        object::delete(id);
        transfer::public_transfer(o, tx_context::sender(ctx))
    }

    public entry fun delete(o: Object) {
        let Object { id, value: _ } = o;
        object::delete(id);
    }
}

//# run Test::M1::create --args 0 @A

//# create-checkpoint 1

//# run-graphql
{
  object(
    address: "@{obj_2_0}"
  ) {
    status
    version
    asMoveObject {
      contents {
        json
      }
    }
  }
}


//# run-graphql
{
  object(
    address: "@{obj_2_0}"
    version: 3
  ) {
    status
    version
    asMoveObject {
      contents {
        json
      }
    }
  }
}

//# run Test::M1::wrap --sender A --args object(2,0)

//# create-checkpoint

//# run Test::M1::create --args 0 @A

//# create-checkpoint

//# run Test::M1::create --args 0 @A

//# create-checkpoint

//# run Test::M1::create --args 0 @A

//# create-checkpoint

//# run Test::M1::create --args 0 @A

//# create-checkpoint

//# run Test::M1::create --args 0 @A

//# create-checkpoint

//# run Test::M1::create --args 0 @A

//# run-graphql --force-objects-snapshot
# should not exist on live objects
{
  object(
    address: "@{obj_2_0}"
  ) {
    status
    version
    asMoveObject {
      contents {
        json
      }
    }
  }
}


//# run-graphql
# fetched from objects_snapshot
{
  object(
    address: "@{obj_2_0}"
    version: 4
  ) {
    status
    version
    asMoveObject {
      contents {
        json
      }
    }
  }
}

//# run-graphql
# should not exist in either objects_snapshot or objects_history
{
  object(
    address: "@{obj_2_0}"
    version: 3
  ) {
    status
    version
    asMoveObject {
      contents {
        json
      }
    }
  }
}
