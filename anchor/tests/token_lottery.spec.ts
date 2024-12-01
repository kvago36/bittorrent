import * as anchor from "@coral-xyz/anchor";
import * as sb from "@switchboard-xyz/on-demand";
import { Program } from "@coral-xyz/anchor";
import { TokenLottery } from "../target/types/token_lottery";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { PublicKey, Keypair } from "@solana/web3.js";
import SwitchboardIDL from '../switchboard.json';

describe("basic", () => {
  const provider = anchor.AnchorProvider.env();
  const connection = provider.connection;

  // Configure the client to use the local cluster.
  anchor.setProvider(provider);

  const switchboardProgram = new anchor.Program(SwitchboardIDL as anchor.Idl, provider);

  const wallet = provider.wallet as anchor.Wallet;

  const rngKp = anchor.web3.Keypair.generate();

  const program = anchor.workspace.TokenLottery as Program<TokenLottery>;

  // beforeAll(async () => {
  //   const switchboardIDL = await anchor.Program.fetchIdl(
  //     sb.SB_ON_DEMAND_PID,
  //     {connection: new anchor.web3.Connection("https://api.mainnet-beta.solana.com")}
  //   );

  //   console.log(switchboardIDL)

  //   var fs = require('fs');
  //   fs.writeFileSync("switchboard.json", JSON.stringify(switchboardIDL), function(err: any) {
  //     if (err) {
  //         console.log(err);
  //     }
  //   });

  //   switchboardProgram = new anchor.Program(switchboardIDL as anchor.Idl, provider);
  // });

  async function buyTicket() {
    const buyTicketIx = await program.methods
      .buyTicket()
      .accounts({ tokenProgram: TOKEN_PROGRAM_ID })
      .instruction();

    const blockhashWithContext = await provider.connection.getLatestBlockhash();

    const computeIx = anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
      units: 300000,
    });

    const priorityIx = anchor.web3.ComputeBudgetProgram.setComputeUnitPrice({
      microLamports: 1,
    });

    const tx = new anchor.web3.Transaction({
      feePayer: provider.wallet.publicKey,
      blockhash: blockhashWithContext.blockhash,
      lastValidBlockHeight: blockhashWithContext.lastValidBlockHeight,
    })
      .add(buyTicketIx)
      .add(computeIx)
      .add(priorityIx);

    const signature = await anchor.web3.sendAndConfirmTransaction(
      provider.connection,
      tx,
      [wallet.payer],
      { skipPreflight: true }
    );

    console.log("Buy ticket transaction signature", signature);
  }

  it("should test token lottery", async () => {
    // Add your test here.
    const slot = await provider.connection.getSlot();
    const endSlot = slot + 20;

    const initConfigIx = await program.methods
      .initializeConfig(
        new anchor.BN(0),
        new anchor.BN(1822712025),
        new anchor.BN(10000)
      )
      .instruction();

    const blockhashWithContext = await provider.connection.getLatestBlockhash();

    const tx = new anchor.web3.Transaction({
      feePayer: provider.wallet.publicKey,
      blockhash: blockhashWithContext.blockhash,
      lastValidBlockHeight: blockhashWithContext.lastValidBlockHeight,
    }).add(initConfigIx);

    const signature = await anchor.web3.sendAndConfirmTransaction(
      provider.connection,
      tx,
      [wallet.payer],
      { skipPreflight: true }
    );

    console.log("Your transaction signature", signature);

    const initLotteryIx = await program.methods
      .initializeLottery()
      .accounts({ tokenProgram: TOKEN_PROGRAM_ID })
      .instruction();

    const initLotteryTx = new anchor.web3.Transaction({
      feePayer: provider.wallet.publicKey,
      blockhash: blockhashWithContext.blockhash,
      lastValidBlockHeight: blockhashWithContext.lastValidBlockHeight,
    }).add(initLotteryIx);

    const initLotterySignature = await anchor.web3.sendAndConfirmTransaction(
      provider.connection,
      initLotteryTx,
      [wallet.payer],
      { skipPreflight: true }
    );

    console.log("Your init lottery signature", initLotterySignature);

    await buyTicket();
    await buyTicket();
    await buyTicket();
    await buyTicket();
    await buyTicket();
    await buyTicket();
    await buyTicket();
    await buyTicket();
    await buyTicket();


    const queue = new anchor.web3.PublicKey("A43DyUGA7s8eXPxqEjJY6EBu1KKbNgfxF8h17VAHn13w");

    const queueAccount = new sb.Queue(switchboardProgram, queue);
    console.log("Queue account", queue.toString());
    try {
      await queueAccount.loadData();
    } catch (err) {
      console.log("Queue account not found");
      process.exit(1);
    }

    const [randomness, ix] = await sb.Randomness.create(switchboardProgram, rngKp, queue);
    console.log("Created randomness account..");
    // console.log("Randomness account", randomness.pubkey.toBase58());
    // console.log("rkp account", rngKp.publicKey.toBase58());
    const createRandomnessTx = await sb.asV0Tx({
      connection: connection,
      ixs: [ix],
      payer: wallet.publicKey,
      signers: [wallet.payer, rngKp],
      computeUnitPrice: 75_000,
      computeUnitLimitMultiple: 1.3,
    });
    const blockhashContext = await connection.getLatestBlockhashAndContext();
  
    const createRandomnessSignature = await connection.sendTransaction(createRandomnessTx);
    await connection.confirmTransaction({
      signature: createRandomnessSignature,
      blockhash: blockhashContext.value.blockhash,
      lastValidBlockHeight: blockhashContext.value.lastValidBlockHeight
    });
    console.log(
      "Transaction Signature for randomness account creation: ",
      createRandomnessSignature
    );

    let confirmed = false;

    while (!confirmed) {
      try {
        const confirmedRandomness = await provider.connection.getSignatureStatuses([createRandomnessSignature]);
        const randomnessStatuts = confirmedRandomness.value[0];
        if (randomnessStatuts?.confirmations != null && randomnessStatuts?.confirmationStatus === "confirmed") {
          confirmed = true
        }
      } catch (err) {
        console.log("Error while confirming transaction", err);
      }
    }

    const sbCommitIx = await randomness.commitIx(queue);

    const commitix = await program.methods.commitRandomness().accounts({
      randomnessAccount: randomness.pubkey,
    }).instruction();

    const commitComputIx = anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
      units: 100000,
    });
    
    const commitPriorityIx = anchor.web3.ComputeBudgetProgram.setComputeUnitPrice({
      microLamports: 1,
    });

    const commitBlockhashWithContext = await provider.connection.getLatestBlockhash();
    const commitTx = new anchor.web3.Transaction({
      feePayer: provider.wallet.publicKey,
      blockhash: commitBlockhashWithContext.blockhash,
      lastValidBlockHeight: commitBlockhashWithContext.lastValidBlockHeight,
    })
      .add(commitComputIx)
      .add(commitPriorityIx)
      .add(sbCommitIx)
      .add(commitix);

    const commitSignature = await anchor.web3.sendAndConfirmTransaction(connection, commitTx, [wallet.payer], { skipPreflight: true });

    console.log("Commit randomness transaction signature", commitSignature);

    const sbRevealIx = await randomness.revealIx();

    const revealWinnerIx = await program.methods.revealWinner().accounts({
      randomnessAccount: randomness.pubkey,
    }).instruction();

    const revealBlockWithContex = await provider.connection.getLatestBlockhash();

    const revealTx = new anchor.web3.Transaction({
      feePayer: provider.wallet.publicKey,
      blockhash: revealBlockWithContex.blockhash,
      lastValidBlockHeight: revealBlockWithContex.lastValidBlockHeight,
    }).add(sbRevealIx).add(revealWinnerIx);

    let currentSlot = 0;

    while (currentSlot < endSlot) {
      const slot = await provider.connection.getSlot();

      if (slot > currentSlot) {
        currentSlot = slot;
        console.log("Current slot", currentSlot);
      }
    }

    const revealSignature = await anchor.web3.sendAndConfirmTransaction(
      provider.connection,
      revealTx,
      [wallet.payer],
      { skipPreflight: true }
    );

    console.log("Transaction Signature revealTx", revealSignature);
  }, 300000);
});
