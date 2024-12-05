use bioma_actor::prelude::*;
use futures::StreamExt;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
enum PlayerType {
    X,
    O,
    Null,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StartGame;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MakeMove {
    player: PlayerType,
    position: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GameState {
    // #[serde(default = "default_board")]
    board: Vec<PlayerType>,
    current_player: PlayerType,
    game_over: bool,
    #[serde(default)]
    winner: Option<PlayerType>,
}

// fn default_board() -> [Option<PlayerType>; 9] {
//     [None; 9]
// }

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GameResult {
    winner: Option<PlayerType>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GameActor {
    player_x: ActorId,
    player_o: ActorId,
    board: ActorId,
    current_player: PlayerType,
}

impl Message<StartGame> for GameActor {
    type Response = ();

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, _: &StartGame) -> Result<Self::Response, Self::Error> {
        let current_player = if self.current_player == PlayerType::X { &self.player_x } else { &self.player_o };

        let game_state = GameState {
            board: vec![PlayerType::Null; 9],
            current_player: self.current_player,
            game_over: false,
            winner: None,
        };
        info!("{}: Sending GameState to player: {:?}", ctx.id(), self.current_player);
        ctx.do_send::<PlayerActor, GameState>(game_state, current_player).await?;
        Ok(())
    }
}

impl Message<GameResult> for GameActor {
    type Response = ();

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        result: &GameResult,
    ) -> Result<Self::Response, Self::Error> {
        info!("{} {:?}", ctx.id(), result);
        ctx.do_send::<PlayerActor, GameResult>(result.clone(), &self.player_x).await?;
        ctx.do_send::<PlayerActor, GameResult>(result.clone(), &self.player_o).await?;
        Ok(())
    }
}

impl Actor for GameActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(game_result) = frame.is::<GameResult>() {
                self.reply(ctx, &game_result, &frame).await?;
                break;
            } else if let Some(start_game) = frame.is::<StartGame>() {
                self.reply(ctx, &start_game, &frame).await?;
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PlayerActor {
    player_type: PlayerType,
    game: ActorId,
    board: ActorId,
}

impl Message<GameState> for PlayerActor {
    type Response = ();

    async fn handle(&mut self, ctx: &mut ActorContext<Self>, state: &GameState) -> Result<Self::Response, Self::Error> {
        let player_type = self.player_type;
        let board = self.board.clone();
        let mut state = state.clone();
        info!("{} Analyzing GameState", ctx.id());
        if state.current_player == player_type {
            // let empty_positions = &state.board;

            if state.board.is_empty() {
                state.board = vec![PlayerType::Null; 9];
            }

            let empty_positions: Vec<usize> = state
                .board
                .iter()
                .enumerate()
                .filter_map(|(i, &cell)| if cell.eq(&PlayerType::Null) { Some(i) } else { None })
                .collect();

            if !empty_positions.is_empty() {
                let mut rng = StdRng::from_entropy();
                let random_position = empty_positions[rng.gen_range(0..empty_positions.len())];
                info!("{} Making move at position {}", ctx.id(), random_position);
                ctx.do_send::<BoardActor, MakeMove>(
                    MakeMove { player: player_type, position: random_position },
                    &board,
                )
                .await?;
            } else {
                info!("{} No empty positions available", ctx.id());
            }
        }
        Ok(())
    }
}

impl Message<GameResult> for PlayerActor {
    type Response = ();

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        result: &GameResult,
    ) -> Result<Self::Response, Self::Error> {
        match result.winner {
            Some(winner) if winner == self.player_type => info!("{} Player {:?} wins!", ctx.id(), self.player_type),
            Some(_) => info!("{} Player {:?} loses!", ctx.id(), self.player_type),
            None => info!("{} It's a draw!", ctx.id()),
        }
        Ok(())
    }
}

impl Actor for PlayerActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} {:?} started", ctx.id(), self.player_type);
        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(game_state) = frame.is::<GameState>() {
                self.reply(ctx, &game_state, &frame).await?;
            } else if let Some(game_result) = frame.is::<GameResult>() {
                self.reply(ctx, &game_result, &frame).await?;
                break;
            }
        }
        info!("{} {:?} finished", ctx.id(), self.player_type);
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BoardActor {
    game: ActorId,
    player_x: ActorId,
    player_o: ActorId,
    board: Vec<PlayerType>,
    current_player: PlayerType,
    game_over: bool,
}

impl BoardActor {
    fn check_winner(&self) -> Option<PlayerType> {
        const WINNING_COMBINATIONS: [[usize; 3]; 8] = [
            [0, 1, 2],
            [3, 4, 5],
            [6, 7, 8], // Rows
            [0, 3, 6],
            [1, 4, 7],
            [2, 5, 8], // Columns
            [0, 4, 8],
            [2, 4, 6], // Diagonals
        ];

        for combo in &WINNING_COMBINATIONS {
            if let (player, true) = (
                self.board[combo[0]],
                self.board[combo[0]] == self.board[combo[1]] && self.board[combo[1]] == self.board[combo[2]],
            ) {
                if player != PlayerType::Null {
                    return Some(player);
                }
            }
        }

        None
    }

    fn is_full(&self) -> bool {
        self.board.iter().all(|cell| cell.eq(&PlayerType::X) || cell.eq(&PlayerType::O))
    }

    fn draw_board(&self) -> String {
        let mut board_str = String::new();
        for i in 0..3 {
            for j in 0..3 {
                let index = i * 3 + j;
                let symbol = match self.board[index] {
                    PlayerType::X => "X",
                    PlayerType::O => "O",
                    PlayerType::Null => " ",
                };
                board_str.push_str(symbol);
                if j < 2 {
                    board_str.push('|');
                }
            }
            board_str.push('\n');
            if i < 2 {
                board_str.push_str("-+-+-\n");
            }
        }
        board_str
    }
}

impl Message<MakeMove> for BoardActor {
    type Response = ();

    async fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        move_msg: &MakeMove,
    ) -> Result<Self::Response, Self::Error> {
        info!("{} Received MakeMove: player {:?}, position {}", ctx.id(), move_msg.player, move_msg.position);

        // Check if the move is valid
        if move_msg.player == self.current_player && self.board[move_msg.position].eq(&PlayerType::Null) {
            info!("{} Move is valid. Updating board state.", ctx.id());
            self.board[move_msg.position] = move_msg.player;

            info!("{} Current board state:\n{}", ctx.id(), self.draw_board());

            let winner = self.check_winner();
            info!("{} Winner: {:?}", ctx.id(), winner);

            let is_full = self.is_full();
            info!("{} Board full: {}", ctx.id(), is_full);

            self.game_over = winner.is_some() || is_full;
            info!("{} Game over: {}", ctx.id(), self.game_over);

            if self.game_over {
                info!("{} Game is over. Sending GameResult to {}", ctx.id(), &self.game);
                ctx.do_send::<GameActor, GameResult>(GameResult { winner }, &self.game).await?;
            } else {
                info!("{} Game continues. Switching current player.", ctx.id());
                self.current_player = match self.current_player {
                    PlayerType::X => PlayerType::O,
                    PlayerType::O => PlayerType::X,
                    PlayerType::Null => self.current_player,
                };

                info!("{} Current player is now: {:?}", ctx.id(), self.current_player);

                let next_player = if self.current_player == PlayerType::X { &self.player_x } else { &self.player_o };

                info!("{} Sending updated GameState to next player: {:?}", ctx.id(), self.current_player);
                ctx.do_send::<PlayerActor, GameState>(
                    GameState {
                        board: self.board.clone(),
                        current_player: self.current_player,
                        game_over: self.game_over,
                        winner,
                    },
                    next_player,
                )
                .await?;
            }
        } else {
            if move_msg.player != self.current_player {
                error!(
                    "{} Invalid move: Not the current player's turn. Current player: {:?}, Move attempt by: {:?}",
                    ctx.id(),
                    self.current_player,
                    move_msg.player
                );
            } else {
                let player_at_position = self.board[move_msg.position];
                error!(
                    "{} Invalid move: Position {} is already occupied by {:?}",
                    ctx.id(),
                    move_msg.position,
                    player_at_position
                );
            }
        }
        Ok(())
    }
}

impl Actor for BoardActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());

        // Draw the initial empty board
        info!("{} Initial board state:\n{}", ctx.id(), self.draw_board());

        let mut stream = ctx.recv().await?;
        while let Some(Ok(frame)) = stream.next().await {
            if let Some(move_msg) = frame.is::<MakeMove>() {
                self.reply(ctx, &move_msg, &frame).await?;
            }
            if self.game_over {
                break;
            }
        }
        info!("{} Finished", ctx.id());
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MainActor;

impl Actor for MainActor {
    type Error = SystemActorError;

    async fn start(&mut self, ctx: &mut ActorContext<Self>) -> Result<(), Self::Error> {
        info!("{} Started", ctx.id());
        // Create actor IDs
        let game_id = ActorId::of::<GameActor>("/game");
        let board_id = ActorId::of::<BoardActor>("/board");
        let player_x_id = ActorId::of::<PlayerActor>("/player_x");
        let player_o_id = ActorId::of::<PlayerActor>("/player_o");

        // Spawn game actor
        let (mut game_actor_ctx, mut game_actor) = Actor::spawn(
            ctx.engine().clone(),
            game_id.clone(),
            GameActor {
                player_x: player_x_id.clone(),
                player_o: player_o_id.clone(),
                board: board_id.clone(),
                current_player: PlayerType::X,
            },
            SpawnOptions::default(),
        )
        .await?;

        // Spawn board actor

        let (mut board_actor_ctx, mut board_actor) = Actor::spawn(
            ctx.engine().clone(),
            board_id.clone(),
            BoardActor {
                game: game_id.clone(),
                player_x: player_x_id.clone(),
                player_o: player_o_id.clone(),
                board: vec![PlayerType::Null; 9],
                current_player: PlayerType::X,
                game_over: false,
            },
            SpawnOptions::default(),
        )
        .await?;

        // Spawn player_x actor
        let (mut player_x_actor_ctx, mut player_x_actor) = Actor::spawn(
            ctx.engine().clone(),
            player_x_id.clone(),
            PlayerActor { player_type: PlayerType::X, game: game_id.clone(), board: board_id.clone() },
            SpawnOptions::default(),
        )
        .await?;

        // Spawn player_o actor
        let (mut player_o_actor_ctx, mut player_o_actor) = Actor::spawn(
            ctx.engine().clone(),
            player_o_id.clone(),
            PlayerActor { player_type: PlayerType::O, game: game_id.clone(), board: board_id.clone() },
            SpawnOptions::default(),
        )
        .await?;

        // Start the game actor
        let game_handle = tokio::spawn(async move {
            game_actor.start(&mut game_actor_ctx).await.unwrap();
        });

        // Start the board actor
        let board_handle = tokio::spawn(async move {
            board_actor.start(&mut board_actor_ctx).await.unwrap();
        });

        // Start the player_x actor
        let player_x_handle = tokio::spawn(async move {
            player_x_actor.start(&mut player_x_actor_ctx).await.unwrap();
        });

        // Start the player_o actor
        let player_o_handle = tokio::spawn(async move {
            player_o_actor.start(&mut player_o_actor_ctx).await.unwrap();
        });

        // Start the game actor
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        ctx.do_send::<GameActor, StartGame>(StartGame, &game_id).await?;

        info!("{} Started game", ctx.id());

        // Wait for all actors to finish
        let _ = game_handle.await;
        let _ = board_handle.await;
        let _ = player_x_handle.await;
        let _ = player_o_handle.await;

        info!("{} Finished", ctx.id());
        Ok(())
    }
}

#[tokio::main(worker_threads = 10)]
async fn main() -> Result<(), SystemActorError> {
    let filter = match tracing_subscriber::EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => tracing_subscriber::EnvFilter::new("info"),
    };

    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    color_backtrace::install();
    color_backtrace::BacktracePrinter::new().message("BOOM! 💥").install(color_backtrace::default_output_stream());

    let engine = Engine::test().await?;
    // let engine_options = EngineOptions::builder().endpoint("ws://localhost:8000".into()).build();
    // let engine = Engine::connect(engine_options).await?;

    // Setup the main actor
    let (mut main_actor_ctx, mut main_actor) =
        Actor::spawn(engine.clone(), ActorId::of::<MainActor>("/main"), MainActor, SpawnOptions::default()).await?;

    // Start the main actor with a timeout
    // and wait for the main actor to finish or timeout after 10 seconds
    let main_timeout = std::time::Duration::from_secs(10);
    let _result = tokio::time::timeout(main_timeout, main_actor.start(&mut main_actor_ctx))
        .await
        .map_err(|_| SystemActorError::TaskTimeout(main_timeout))?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
