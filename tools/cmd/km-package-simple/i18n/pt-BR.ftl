# km-package-simple: every word its pages say, in Brazilian Portuguese.
#
# A message with a `{ $variable }` is filled in by the Rust. A template names only messages with
# none. A count carries a `[0]` variant, because Portuguese files zero under `one`.

app-title = KaraokeMachine Simple Package Builder
language-picker = Idioma
action-quit = Sair

home-heading = Criar um pacote a partir de uma pasta
home-intro = Escolha uma pasta de arquivos de karaokê. Cada arquivo MIDI, vídeo, par MP3+G e música UltraStar nela e nas subpastas vira uma música do pacote. Você pode renomear músicas e deixar algumas de fora antes de criar o pacote.
home-uncurated = Um pacote criado aqui é marcado como sem curadoria, porque ninguém revisou as músicas. As listas de pacotes da máquina mostram a marca. A televisão não mostra.
home-folder = Pasta
home-folder-placeholder = O caminho completo de uma pasta
action-read = Ler a pasta
action-other-folder = Escolher outra pasta

said-failed = Não funcionou:
said-busy = Espere o trabalho em andamento terminar.
said-no-folder = Não há pasta nesse caminho.
said-nothing-kept = Mantenha pelo menos uma música.
said-no-name = Dê um nome ao pacote.
said-reveal-failed = Não foi possível abrir a pasta.

progress-reading = Lendo a pasta: { $done } de { $total } arquivos MIDI.
progress-building = Escrevendo o pacote { $volume } de { $volumes }: música { $done } de { $total }.

form-uncurated = O pacote é marcado como sem curadoria.
form-name = Nome
form-version = Versão
form-publisher = Editor
form-language = Idioma das músicas sem idioma
form-out = Gravar na pasta
form-language-hint = Uma música cujo arquivo não diz o idioma fica neste. Indeterminado é a resposta honesta quando você não sabe.
action-build = Criar

songs-summary = { $count ->
    [0] { $kept } de { $count } músicas entram
    [one] { $kept } de { $count } música entra
   *[other] { $kept } de { $count } músicas entram
  }, em { $volumes ->
    [0] { $volumes } pacotes.
    [one] { $volumes } pacote.
   *[other] { $volumes } pacotes.
  }
songs-range = Músicas { $first } a { $last } de { $count }
songs-shift-hint = Clique numa caixa com Shift para manter ou deixar de fora todas as músicas entre ela e a última caixa clicada.
column-number = Número
column-kind = Tipo
column-title = Título
column-artist = Artista
column-language = Idioma
column-suitability = Adequação
column-keep = Manter
page-previous = Anterior
page-next = Próxima

kind-midi = MIDI
kind-video = Vídeo
kind-cdg = MP3+G
kind-ultrastar = UltraStar

left-heading = { $count ->
    [0] { $count } arquivos não são músicas
    [one] { $count } arquivo não é música
   *[other] { $count } arquivos não são músicas
  }
left-unreadable = não pôde ser lido.
left-not-midi = não é um arquivo MIDI que este programa consiga ler.
left-copy = tem a mesma música que { $detail }.
left-ultrastar = é um arquivo UltraStar que este programa não usa: { $detail }
left-no-graphics = não tem um .cdg ao lado, então não tem letra.
left-no-audio = não tem áudio ao lado, então não há o que cantar.
left-other = ficou de fora: { $detail }

built-heading = { $count ->
    [0] { $count } pacotes gravados.
    [one] { $count } pacote gravado.
   *[other] { $count } pacotes gravados.
  }
built-uncurated = Cada um é marcado como sem curadoria.
built-listing = Uma lista das músicas fica ao lado de cada pacote, em um arquivo .txt com o mesmo nome.
built-skipped = { $count ->
    [0] { $count } músicas não entraram
    [one] { $count } música não entrou
   *[other] { $count } músicas não entraram
  }
count-songs = { $count ->
    [0] { $count } músicas
    [one] { $count } música
   *[other] { $count } músicas
  }
action-reveal = Mostrar na pasta
action-back = Voltar às músicas
