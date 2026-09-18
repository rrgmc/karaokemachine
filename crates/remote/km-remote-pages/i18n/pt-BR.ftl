# O que o controle do cantor diz.
#
# Tradução de `en.ftl`, que é onde toda mensagem é escrita primeiro.
#
# **Isto é lido num celular, numa festa, por alguém segurando um microfone.** Curto e simples vence
# completo e cuidadoso — um aviso fica quatro segundos na tela.

## Recusas

error-unavailable = A máquina não pode fazer isso agora.
error-queue-full = A fila está cheia.
error-unauthorized = Esta máquina exige uma senha.
browse-back = Voltar para as músicas
error-not-found = Isso não está mais aí.
error-not-acknowledged = Enviado, mas a máquina não confirmou.
error-failed = Não deu certo. Tente de novo.

# Aqui está a razão de `$kind` ser um argumento e não parte da frase: o artigo concorda com o
# substantivo, e `uma música em vídeo` não sai de nenhuma tradução de `a video song has no key`.
error-no-key = { $kind ->
    [midi] Esta música não tem tom para mudar.
    [video] Uma música em vídeo não tem tom para mudar.
    [cdg] Uma música MP3+G não tem tom para mudar.
    [ultrastar] Uma música UltraStar não tem tom para mudar.
   *[other] Esta música não tem tom para mudar.
 }
error-no-tempo = { $kind ->
    [video] Uma música em vídeo não tem ritmo para mudar.
    [cdg] Uma música MP3+G não tem ritmo para mudar.
    [ultrastar] Uma música UltraStar não tem ritmo para mudar.
   *[other] Esta música não tem ritmo para mudar.
 }
error-no-melody = { $kind ->
    [video] Uma música em vídeo não tem melodia guia.
    [cdg] Uma música MP3+G não tem melodia guia.
    [ultrastar] Uma música UltraStar não tem melodia guia.
   *[other] Esta música não tem melodia guia.
 }
error-no-melody-channel = Não foi possível achar o canal de melodia nesta música.

error-nothing-playing = Nada está tocando.
error-nothing-loaded = Nenhuma música está carregada.
error-nothing-queued = A fila está vazia. Escolha uma música primeiro.
error-no-sound = Esta máquina não está produzindo som. Fale com o administrador.

error-no-favorites = Este controle não guarda favoritas.
error-no-demo = Esta máquina não tem modo demonstração.

## A barra de abas
#
# Uma palavra cada, lida de relance embaixo de um ícone.

tab-songs = Músicas
tab-now = Tocando
tab-queue = Fila
tab-setup = Configuração
tab-packages = Pacotes

## Títulos de página


## A lista de músicas

search-clear = Limpar a busca
filter-initial = Primeira letra
filter-language = Idioma
filter-all = Todos
filter-tags = Tags
filter-tag-add = Adicionar tag
filter-tag-remove = Remover tag
filter-tags-clear = Limpar tudo
load-more = Carregar mais
book-link = A lista de músicas, em PDF
youtube-link = Procurar no YouTube
no-lyric-match = não achado nesta música

## As ações de uma música

song-queue = Colocar na fila
song-play-now = Tocar agora
song-play-next = Tocar em seguida
song-favorite = Adicionar às favoritas
song-favorited = Está numa pasta de favoritas
song-unfavorite = Tirar desta pasta
song-more-folders = Adicionar a mais de uma…
row-actions = Controles por música
row-actions-extra = Controles extras

## A fila

queue-move-up = Subir
queue-move-down = Descer
queue-remove = Tirar da fila
queue-empty-hint = A fila está vazia. Ache uma música na aba Músicas e toque em
singing-as = Cantando como
singer-name = Nome com que suas músicas entram na fila
singer-placeholder = seu nome

## Tocando agora

transport-show = Mostrar controles
transport-play = Tocar
transport-pause = Pausar
transport-stop = Parar
transport-skip = Pular para a próxima
transport-restart = Reiniciar a música
control-key = Tom
control-key-up = Subir tom
control-key-down = Descer tom
control-tempo = Ritmo
control-faster = Mais rápido
control-slower = Mais devagar
control-music = Música
control-volume = Volume da música
control-melody = Melodia guia
nobody-singing = Música de demonstração
play-something = Iniciar
demo-hint = Modo demonstração — escolha uma música e ela começa na hora
pinned = fixado
reset = voltar ao normal

## Favoritas

folder-new = Nova pasta…
folder-new-name = Nome da nova pasta
folder-add = Adicionar
folder-done = Pronto
folder-close = Fechar

## A máquina

tab-machine = Máquina
tab-book = Livro
machine-none = Nenhuma máquina selecionada
machine-address = O endereço da máquina
machine-address-example = 192.168.1.5
machine-save = Salvar
machine-use = Usar esta
machine-use-instead = Usar esta no lugar
machine-change = Trocar…
machine-rescan = Procurar novamente
machine-also-on-network = Também na rede:
machine-open-browser = Abrir no navegador
machine-open-browser-title = abrir o controle padrão desta máquina no seu navegador
connected = Conectado à máquina de karaokê
not-connected = Sem conexão
reconnecting = Reconectando…
went-wrong = Alguma coisa deu errado.



## Idioma

language-picker = Idioma

## Os modos de busca

mode-songs = Músicas
mode-artists = Artistas
# Um símbolo, não uma palavra. Não traduzir.
mode-favorites = ★
search-placeholder-songs = Música ou número
search-placeholder-artists = Artista
search-placeholder-favorites = Pasta

## O que a lista diz sobre si mesma

# `mostrando 112` em vez de `112 mostradas`: a lista pode ser de músicas, de artistas ou de pastas,
# e um particípio concordaria com uma delas e erraria as outras.

list-count-shown = mostrando { $count }
list-count-of = { $showing } de { $total }

empty-songs = Nenhuma música. Algum pacote está instalado?
empty-search = Nada corresponde a “{ $query }”.
empty-initial = Nenhuma música começa com { $initial }.
empty-initial-search = Nada que comece com { $initial } corresponde a “{ $query }”.
empty-folder = { $folder } está vazia.
empty-folder-initial = Nada em { $folder } começa com { $initial }.
empty-folder-search = Nada em { $folder } corresponde a “{ $query }”.
empty-artists = Nenhum artista. Algum pacote está instalado?
empty-artists-search = Nenhum artista corresponde a “{ $query }”.
empty-artists-initial = Nenhum artista começa com { $initial }.
empty-artists-initial-search = Nenhum artista que comece com { $initial } corresponde a “{ $query }”.
empty-folders = Nenhuma favorita ainda. Toque na ☆ de uma música para criar uma pasta.
empty-folders-search = Nenhuma pasta corresponde a “{ $query }”.

## O que está tocando

now-nothing-playing = Nada tocando
now-up-next = A seguir
now-singer-for = para

## O cartão da máquina, e a faixa acima dele

machine-how-asked-for = pedida
machine-how-remembered = lembrada
machine-how-moved = mesma máquina, endereço novo
machine-how-adopted = achada na rede
machine-how-chosen = escolhida

machine-connected = Conectado
machine-rescan-button = Procurar de novo na rede
machine-refresh = Atualizar a lista de músicas
machine-pin-hint = Uma máquina informada aqui fica salva mesmo quando está desligada. Procure de novo para buscar na rede.
banner-unreachable = A máquina de karaokê não está acessível.
banner-retrying = tentando

machine-songs-copied =
    { $count ->
        [one] { $count } música nesta cópia
       *[other] { $count } músicas nesta cópia
    }

## Contando músicas

count-songs =
    { $count ->
        [one] { $count } música
       *[other] { $count } músicas
    }

## Miudezas

toggle-on = Ligada
toggle-off = Desligada

song-unfavorite-confirm = Tirar “{ $title }” desta pasta?

## O que o botão de uma linha deixa para trás

badge-queued = na fila
badge-next-up = é a próxima
badge-playing = tocando
badge-sent = enviada

## O que uma ação numa música diz

queued-song = Na fila: { $title }
playing-next-song = Toca em seguida: { $title }
playing-now-song = Tocando agora: { $title }
queued-not-moved = { $title } entrou na fila, mas não deu para enviar.
next-not-started = { $title } é a próxima, mas não deu para iniciar.
demo-starting = Começando uma música…

## Escolhendo a máquina

machine-not-chooseable = Este controle não pode trocar de máquina.
machine-type-address = Digite um endereço primeiro.
machine-offer-gone = Essa máquina não está mais disponível.
machine-found-named = Achamos { $name } em { $url }.
machine-found = Achamos { $url }.
machine-scan-nothing = Nenhuma máquina respondeu na rede. Continuando no mesmo endereço.
machine-scan-already = A máquina encontrada na rede é a que você já está usando.
machine-scan-kept = Continuando na mesma máquina.

machine-more-answered =
    { $count ->
        [one] Mais uma respondeu.
       *[other] Mais { $count } responderam.
    }


singer-set = Suas músicas vão entrar na fila como { $name }.
singer-cleared = Suas músicas vão entrar na fila sem nome.
packages-shown = Pacotes na minha lista de músicas
packages-shown-hint = Um pacote desmarcado fica fora das buscas, dos artistas e dos filtros neste celular. O número da música e os seus favoritos ainda encontram as músicas dele.
packages-hidden-saved = Sua lista de músicas deixa de fora os pacotes desmarcados.
packages-all-shown = Sua lista de músicas mostra todos os pacotes.
empty-hidden-packages = Os pacotes que você ocultou em Configuração, Pacotes não entram na busca.
folder-needs-name = Digite um nome.
folder-made = Pasta { $folder } criada.
folder-renamed = Renomeada para { $folder }.
folder-deleted = Pasta apagada.
favorite-added = Adicionada a { $folder }.
favorite-removed = Removida de { $folder }.
favorite-removed-short = Tirada.

## O que este aparelho diz de si mesmo

offline-not-answering = A máquina de karaokê não está respondendo.
offline-none-found = Nenhuma máquina de karaokê foi achada ainda.
offline-stream-closed = A máquina de karaokê encerrou a conexão.
connection-looking = Procurando uma máquina de karaokê…
connection-connecting = Conectando…
folder-name-taken = Já existe uma pasta com esse nome.
folder-only-one = Esta é a única pasta. Renomeie-a em vez de apagar.

share-error-format = Isso não é um código de favoritas.
share-error-damaged = Esse código está danificado. Leia ou copie ele de novo.
share-error-empty = Não tem nenhuma música nesta pasta para compartilhar.
share-error-too-large = Esta pasta é grande demais para um código só. Copie o texto em vez disso.

backup-error-not-ours = Esse não é um arquivo de favoritas.
backup-error-damaged = Não foi possível ler esse arquivo.
backup-error-empty = Não tem nenhuma música nesse arquivo.
backup-error-too-large = Esse arquivo é grande demais para ser um backup de favoritas.
backup-error-no-file = Nenhum arquivo foi escolhido.

step-back = Voltar
step-exit = Voltar para as favoritas

share-link = Compartilhar
backup-link = Backup
backup-setup-sub = Salvar suas favoritas em um arquivo, ou restaurar a partir de um

owner-page-link = Configurar esta máquina
owner-page-sub = Músicas, imagens, som e o nome dela. Pede a senha da máquina

share-send = Enviar
share-send-sub = Mostrar um código para o outro aparelho ler
share-receive = Receber
share-title = Compartilhar { $folder }
share-receive-sub = Ler um código e adicionar as músicas dele em { $folder }
share-one-way =
    As músicas só são adicionadas — nada é removido em nenhum dos aparelhos, e ler o mesmo código
    duas vezes não muda nada. Isso leva { $folder } em uma direção só, então faça nos dois sentidos
    para os dois aparelhos ficarem iguais.

share-code-hint = No outro aparelho, abra a mesma pasta, escolha Receber e aponte para isso.
share-cant-scan = Não consegue ler?
share-code-copy = Copie isso e cole em Receber no outro aparelho.
share-too-dense = Músicas demais para um código só. Copie o texto abaixo em vez disso.
share-code-alt = Código de { $folder }

share-camera-start = Ligar a câmera
share-camera-stop = Desligar a câmera
share-camera-starting = Ligando a câmera…
share-camera-got-it = Código lido…
share-camera-refused = A câmera não foi permitida. Ligue ela para este app nas configurações do sistema, ou use “Não consegue ler?” abaixo.
share-camera-none = Nenhuma câmera foi encontrada. Use “Não consegue ler?” abaixo.
share-camera-failed = Não foi possível ligar a câmera. Use “Não consegue ler?” abaixo.
share-point-at = Aponte para o código mostrado no outro aparelho. As músicas dele vão para { $folder }.
share-paste-hint = Cole o código mostrado embaixo do código do outro aparelho.
share-continue = Continuar

share-confirm-add = Adicionar em { $folder }
share-confirm-mismatch = Este código veio de { $from }, e você está adicionando em { $into }.
share-scan-another = Ler outro código

favorites-added =
    { $added ->
        [one] { $added } música adicionada
       *[other] { $added } músicas adicionadas
    }
favorites-already-here =
    { $count ->
        [one] { $count } já estava aqui
       *[other] { $count } já estavam aqui
    }
favorites-left-out =
    { $count ->
        [one] Uma música não está na lista deste aparelho, então ela ficou de fora.
       *[other] { $count } músicas não estão na lista deste aparelho, então elas ficaram de fora.
    }
favorites-not-here =
    { $count ->
        [one] Uma música desta pasta não está nesta máquina.
       *[other] { $count } músicas desta pasta não estão nesta máquina.
    }
favorites-missing-package =
    { $count ->
        [one] Uma música vem de um pacote que este aparelho não tem: { $codes }.
       *[other] { $count } músicas vêm de pacotes que este aparelho não tem: { $codes }.
    }
favorites-missing-recording =
    { $count ->
        [one] Uma música não está em nenhum pacote daqui: { $codes }.
       *[other] { $count } músicas não estão em nenhum pacote daqui: { $codes }.
    }
count-folders =
    { $count ->
        [one] { $count } pasta
       *[other] { $count } pastas
    }
share-open-folder = Abrir { $folder }
share-return-leg =
    Este aparelho já tem elas. Para os dois ficarem iguais, mostre o código desta pasta aqui e
    receba ele no outro.
share-show-code = Mostrar o código desta pasta

backup-title = Backup
backup-save = Salvar um backup
backup-save-sub = { $songs } em { $folders }, em um arquivo só
backup-nothing = Ainda não tem nenhuma favorita aqui, então não tem nada para salvar.
backup-restore = Restaurar de um arquivo
backup-restore-sub = Adicionar o conteúdo de um backup salvo às suas favoritas
backup-add-only =
    Restaurar só adiciona. As pastas que estiverem faltando são criadas, as músicas que já estão
    aqui ficam como estão, e nada é removido — então restaurar o mesmo arquivo duas vezes não muda
    nada na segunda.
backup-restore-title = Restaurar favoritas
backup-choose-hint =
    Escolha um arquivo salvo por Salvar um backup — neste aparelho, ou no Arquivos, no Dropbox ou no
    Drive. Tudo que está nele é adicionado ao que já está aqui; nada é removido, e as pastas que
    estiverem faltando são criadas.
backup-paste = Colar em vez disso
backup-paste-hint = Abra o arquivo de backup, copie tudo que está nele e cole aqui.
backup-restore-button = Restaurar
backup-read-from = { $read } lidas de { $folders }
backup-folders-created =
    { $count ->
        [one] { $count } pasta criada
       *[other] { $count } pastas criadas
    }
backup-unreadable =
    { $count ->
        [one] Uma linha do arquivo não era um número de música, então ela foi ignorada.
       *[other] { $count } linhas do arquivo não eram números de música, então elas foram ignoradas.
    }
backup-format-newer = Esse arquivo foi feito por uma versão mais nova. Parte dele não pôde ser lida.
backup-unchanged =
    Nada foi removido, e o arquivo está do mesmo jeito — restaurar ele de novo não adicionaria nada.
backup-open-favorites = Abrir favoritas

machine-now-using = Usando { $url } agora.
machine-copy-current =
    { $count ->
        [one] Já está em dia — { $count } música.
       *[other] Já está em dia — { $count } músicas.
    }
machine-copy-imported =
    { $count ->
        [one] { $count } música copiada da máquina.
       *[other] { $count } músicas copiadas da máquina.
    }
machine-copy-not-answering = Ela ainda não está respondendo. Será usada assim que responder.
