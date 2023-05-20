import * as anchor from "@coral-xyz/anchor"
import { Program } from "@coral-xyz/anchor"
import { Vrf } from "../target/types/vrf"
import * as sbv2 from "@switchboard-xyz/solana.js"
import { NodeOracle } from "@switchboard-xyz/oracle"

describe("vrf", () => {
  const provider = anchor.AnchorProvider.env()
  anchor.setProvider(provider)

  const program = anchor.workspace.Vrf as Program<Vrf>
  const wallet = anchor.workspace.Vrf.provider.wallet

  // Keypair used to create new VRF account during setup
  const vrfSecret = anchor.web3.Keypair.generate()
  console.log(`VRF Account: ${vrfSecret.publicKey}`)

  // PDA for VrfClientState Account, VRF Account is authority of this account
  const [vrfClientKey] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("GAME"), wallet.publicKey.toBytes()],
    program.programId
  )
  console.log(`VRF Client: ${vrfClientKey}`)

  // PDA for VrfClientState Account, VRF Account is authority of this account
  const [solVaultPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("VAULT")],
    program.programId
  )

  console.log(`Sol Vault PDA: ${solVaultPDA}`)

  const vrfIxCoder = new anchor.BorshInstructionCoder(program.idl)

  // Callback to consume randomness (the instruction that the oracle CPI's back into our program)
  const vrfClientCallback: sbv2.Callback = {
    programId: program.programId,
    accounts: [
      // ensure all accounts in consumeRandomness are populated
      { pubkey: wallet.publicKey, isSigner: false, isWritable: true },
      { pubkey: solVaultPDA, isSigner: false, isWritable: true },
      { pubkey: vrfClientKey, isSigner: false, isWritable: true },
      { pubkey: vrfSecret.publicKey, isSigner: false, isWritable: false },
      {
        pubkey: anchor.web3.SystemProgram.programId,
        isSigner: false,
        isWritable: false,
      },
    ],
    ixData: vrfIxCoder.encode("consumeRandomness", ""), // pass any params for instruction here
  }

  let oracle: NodeOracle
  let vrfAccount: sbv2.VrfAccount

  // // use this for localnet
  // let switchboard: sbv2.SwitchboardTestContext

  // use this for devnet
  let switchboard: {
    program: sbv2.SwitchboardProgram
    queue: sbv2.QueueAccount
  }

  before(async () => {
    // use this for devnet
    const switchboardProgram = await sbv2.SwitchboardProgram.fromProvider(
      provider
    )
    const [queueAccount, queue] = await sbv2.QueueAccount.load(
      switchboardProgram,
      "uPeRMdfPmrPqgRWSrjAnAkH78RqAhe5kXoW6vBYRqFX"
    )
    switchboard = { program: switchboardProgram, queue: queueAccount }
    // // use this for localnet
    // switchboard = await sbv2.SwitchboardTestContext.loadFromProvider(provider, {
    //   name: "Test Queue",
    //   keypair: sbv2.SwitchboardTestContextV2.loadKeypair(
    //     "~/.keypairs/queue.json"
    //   ),
    //   queueSize: 10,
    //   reward: 0,
    //   minStake: 0,
    //   oracleTimeout: 900,
    //   unpermissionedFeeds: true,
    //   unpermissionedVrf: true,
    //   enableBufferRelayers: true,
    //   oracle: {
    //     name: "Test Oracle",
    //     enable: true,
    //     stakingWalletKeypair: sbv2.SwitchboardTestContextV2.loadKeypair(
    //       "~/.keypairs/oracleWallet.json"
    //     ),
    //   },
    // })
    // oracle = await NodeOracle.fromReleaseChannel({
    //   chain: "solana",
    //   releaseChannel: "testnet",
    //   network: "devnet", // disables production capabilities like monitoring and alerts
    //   rpcUrl: switchboard.program.connection.rpcEndpoint,
    //   oracleKey: switchboard.oracle.publicKey.toBase58(),
    //   secretPath: switchboard.walletPath,
    //   silent: true, // set to true to suppress oracle logs in the console
    //   envVariables: {
    //     VERBOSE: "1",
    //     DEBUG: "1",
    //     DISABLE_NONCE_QUEUE: "1",
    //     DISABLE_METRICS: "1",
    //   },
    // })
    // await oracle.startAndAwait()
  })

  // after(async () => {
  //   oracle?.stop()
  // })

  it("Init Player", async () => {
    const queue = await switchboard.queue.loadData()

    // Create Switchboard VRF and Permission account
    ;[vrfAccount] = await switchboard.queue.createVrf({
      callback: vrfClientCallback,
      authority: vrfClientKey, // vrf authority
      vrfKeypair: vrfSecret,
      enable: !queue.unpermissionedVrfEnabled, // only set permissions if required
    })

    const tx = await program.methods
      .initialize()
      .accounts({
        player: wallet.publicKey,
        gameState: vrfClientKey,
        vrf: vrfAccount.publicKey,
      })
      .rpc()
    console.log("Your transaction signature", tx)
  })

  it("request_randomness", async () => {
    const queue = await switchboard.queue.loadData()
    const vrf = await vrfAccount.loadData()

    // derive the existing VRF permission account using the seeds
    const [permissionAccount, permissionBump] = sbv2.PermissionAccount.fromSeed(
      switchboard.program,
      queue.authority,
      switchboard.queue.publicKey,
      vrfAccount.publicKey
    )

    // 0.002 wSOL fee for requesting randomness
    const [payerTokenWallet] =
      await switchboard.program.mint.getOrCreateWrappedUser(
        switchboard.program.walletPubkey,
        { fundUpTo: 0.002 }
      )

    // Request randomness
    const tx = await program.methods
      .requestRandomness(
        permissionBump,
        switchboard.program.programState.bump,
        1
      )
      .accounts({
        player: wallet.publicKey,
        solVault: solVaultPDA,
        gameState: vrfClientKey,
        vrf: vrfAccount.publicKey,
        oracleQueue: switchboard.queue.publicKey,
        queueAuthority: queue.authority,
        dataBuffer: queue.dataBuffer,
        permission: permissionAccount.publicKey,
        escrow: vrf.escrow,
        programState: switchboard.program.programState.publicKey,
        switchboardProgram: switchboard.program.programId,
        payerWallet: payerTokenWallet,
        recentBlockhashes: anchor.web3.SYSVAR_RECENT_BLOCKHASHES_PUBKEY,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .rpc()

    console.log("Your transaction signature", tx)

    const balance = await provider.connection.getBalance(solVaultPDA)
    console.log(`Sol Vault Balance: ${balance}`)

    const result = await vrfAccount.nextResult(
      new anchor.BN(vrf.counter.toNumber() + 1),
      45_000
    )
    if (!result.success) {
      throw new Error(`Failed to get VRF Result: ${result.status}`)
    }

    const vrfClientState = await program.account.gameState.fetch(vrfClientKey)
    console.log(`VrfClient Result: ${vrfClientState.result.toString(10)}`)
    const balance2 = await provider.connection.getBalance(solVaultPDA)
    console.log(`Sol Vault Balance: ${balance2}`)

    const callbackTxnMeta = await vrfAccount.getCallbackTransactions()
    console.log(
      JSON.stringify(
        callbackTxnMeta.map((tx) => tx.meta.logMessages),
        undefined,
        2
      )
    )
  })

  it("close", async () => {
    const tx = await program.methods
      .close()
      .accounts({
        player: wallet.publicKey,
        gameState: vrfClientKey,
      })
      .rpc()
    console.log("Your transaction signature", tx)
  })
})
