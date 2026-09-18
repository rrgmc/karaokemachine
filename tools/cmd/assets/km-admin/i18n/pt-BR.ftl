# As páginas do próprio KaraokeMachine Admin, em português do Brasil.
#
# **Linguagem simples**, que é a regra com que este arquivo começa porque todo catálogo do projeto
# começa assim: `What a user reads is written in plain application language`. Quem lê aqui é a pessoa
# que está preparando uma máquina em um computador, então o registro é o de quem opera — mais simples
# que o controle do cantor e não mais técnico do que a tarefa exige.
#
# **Só a metade deste programa.** *Esta máquina*, Músicas, Imagens e Som são marcação do
# `km-admin-pages` e vêm do catálogo daquele crate. O que está aqui é a porta de entrada, a busca de
# imagens e a busca de bancos — a metade que `A fourth program, rather than a fourth tab on the
# owner's page` mantém fora da máquina.
#
# **Os dados ficam como chegam.** A nota *como ele soa* de um banco e sua licença vêm do `km-banks`;
# os termos de um provedor e o crédito de uma fotografia vêm do provedor. São valores, como o título
# de uma música no controle, e traduzi-los não é tarefa deste arquivo.

## A porta de entrada --------------------------------------------------------

door-heading = Qual máquina?
door-password = A senha da máquina
door-explained = Tudo o que este programa envia vai para a máquina escolhida aqui. Ela precisa estar ligada, para poder conferir a senha.
door-chosen-unnamed = A última máquina escolhida
door-last-used = usada por último
door-typed = Outro endereço
door-address-example = 192.168.1.50
door-looking = Procurando máquinas nesta rede…
door-none-found = Nenhuma máquina respondeu. Digite um endereço acima, ou procure de novo quando a máquina estiver ligada.
door-look-again = Procurar de novo
door-this-one = esta
door-use = Usar esta máquina
door-saved-for = Este computador tem a senha de { $machine }.
door-saved-here = Este computador tem a senha desta máquina.
door-already = Este programa já está conectado a esta máquina.
door-retype = Digitar outra senha
door-retype-again = Digitar a senha de novo
door-remember = Lembrar esta senha neste computador
door-remember-open = Neste computador o arquivo é protegido pelo seu perfil de usuário e por nada mais.
door-forget = Esquecer
door-where = A senha é um código de seis dígitos na tela da própria máquina até alguém mudá-la.

door-locale-heading = Em que idioma este programa está
door-locale-save = Salvar
door-locale-note = Isto vale para as páginas que você está lendo, neste computador. O que aparece na tela da máquina é Idioma da tela, na aba Esta máquina.
door-locale-changed = Estas páginas agora estão neste idioma.
door-needs-an-address = Digite um endereço, ou escolha uma máquina.
door-no-machine = Não foi possível usar esse endereço.
door-password-refused = A máquina não aceitou essa senha.
door-needs-a-password = A senha desta máquina é necessária antes que este programa possa ser usado com ela.
door-unreachable = Essa máquina não respondeu. Verifique se ela está ligada e se o endereço está certo.
send-needs-password = Esta máquina pede a senha antes que algo possa ser enviado a ela. Digite-a aqui e envie de novo.
send-needs-machine = Escolha uma máquina antes de enviar algo para uma.
bank-unknown = Não existe esse banco na lista.
bank-not-here = Esse banco não está na pasta deste programa.
bank-remove-failed = Não foi possível remover { $bank }: { $why }
pack-unknown = Não existe esse pacote na pasta deste programa.
pack-remove-failed = Não foi possível remover { $pack }: { $why }
keys-not-remembered = A chave não foi guardada: { $why }
keys-not-forgotten = As chaves não foram apagadas: { $why }
job-busy-search = Uma busca já está em andamento.
job-busy-fetch = Algo já está sendo baixado.
search-needs-terms = Ainda não há nada para procurar.
search-needs-key = { $provider } precisa de uma chave sua antes de responder.

## O que uma tarefa está fazendo ---------------------------------------------

phase-starting = começando
phase-searching = procurando
phase-downloading = baixando
phase-measuring = medindo
phase-building = montando
phase-sending = enviando

job-stopping = parando
job-stop = Parar

## A tabela de bancos --------------------------------------------------------

sound-heading = Som
sound-lead = Um banco de som fornece os instrumentos com que uma música MIDI é tocada. Baixe um aqui e envie-o para a máquina.
sound-note = A nota de cada linha vem de ouvir aquele banco. A licença mostrada é a que o próprio arquivo do banco declara. Os downloads vêm do endereço de cada editor; este projeto não espelha nada. Se a sua máquina tiver conexão própria com a internet, ela mesma pode baixar bancos.

banks-no-answer = A máquina não respondeu, então esta lista não pode mostrar quais bancos ela já tem. Você ainda pode baixar bancos; eles ficam salvos neste computador.

column-bank = Banco
column-size = Tamanho
column-what-its-like = Como ele soa
column-license = Licença

bank-recommended = Recomendado
bank-shortlist = seleção
bank-here = neste computador
bank-on-the-machine = na máquina
bank-by-hand-from-site = à mão, no site dele
bank-by-hand-only = só à mão
bank-too-large = grande demais para enviar — a máquina pode buscar este sozinha

action-get = Baixar
action-send = Enviar
action-send-again = Enviar de novo
action-remove = Remover

bank-remove-confirm = Remover { $bank } da pasta deste programa? A máquina mantém o que já tiver.

## Pacotes neste computador --------------------------------------------------

packs-heading = Pacotes neste computador
packs-empty = Nenhum pacote ainda. Uma busca concluída salva o pacote dela aqui, e ele fica até você removê-lo.
packs-note = Cada pacote é um arquivo zip que você pode enviar para quantas máquinas quiser. Remover um apaga apenas deste computador; uma máquina mantém o que já recebeu.

column-pack = Pacote
column-pictures = Imagens
column-built = Montado

pack-remove-confirm = Remover { $pack } da pasta deste programa? Uma máquina para a qual você já o enviou mantém a cópia dela.

## Procurando imagens --------------------------------------------------------

pictures-heading = Imagens
pictures-lead = Encontre imagens de fundo com contraste suficiente para as letras continuarem legíveis e envie-as para a máquina. O contraste é medido na parte da tela que as letras ocupam, contra a cor com que são desenhadas e o escurecimento que a máquina aplica.

providers-heading = Onde procurar
provider-no-account = não precisa de conta
provider-key-set = chave definida
provider-needs-key = precisa da sua chave
provider-get-key = Obtenha uma em

key-field = Uma chave para o provedor que você escolheu
key-placeholder = deixe em branco para manter a que já está definida
key-remember = Lembrar dela nesta máquina
key-remember-unix = Gravada na pasta deste programa, legível só por você.
key-remember-windows-lead = Gravada na pasta deste programa.
key-remember-windows-warning = O Windows não tem uma configuração por arquivo, só para o dono, que este programa possa aplicar, então ela fica protegida pela sua pasta de perfil e por mais nada.

pack-name-field = Nome do pacote
pack-name-hint = Entra no nome do zip, que também é como a máquina vai chamá-lo. Letras minúsculas, dígitos e traços; qualquer outra coisa é descartada, e o que sobra é mostrado de volta aqui.
pack-name-needed = Dê um nome a este pacote antes de procurar. Ele entra no nome do arquivo, que é como você vai distinguir dois pacotes.

terms-field = O que procurar, um por linha
pages-field = Páginas por termo
count-field = Imagens no pacote
contrast-field = Contraste a alcançar
contrast-hint = 7,0 é WCAG AAA, que é o contraste com que o pacote da própria máquina foi montado.
ask-width-field = Pedir pelo menos (px de largura)
ask-width-hint = O que se pede ao provedor, pela descrição que ele mesmo dá da fotografia.
keep-width-field = Manter pelo menos (px de largura)
keep-width-hint = A largura que o arquivo baixado precisa ter de fato, que muitas vezes é menor do que o provedor informou. Se todos os resultados forem rejeitados como too_small_on_disk, reduza este valor.

action-save = Salvar
action-forget-keys = Esquecer todas as chaves lembradas

search-heading = Procurar
search-note = Só a etapa de busca baixa alguma coisa. Medir e montar reaproveitam o que já foi baixado, então você pode mudar as opções acima e rodar de novo sem procurar de novo.
search-kept = O pacote pronto fica salvo neste computador. Envie-o para uma máquina pela lista abaixo.
action-search = Procurar, medir e montar

review-heading = No que deu
review-verdict = { $chosen } de { $looked_at } { $looked_at ->
        [one] imagem passou
       *[other] imagens passaram
    } pelo critério.
review-may-pass-on = este pacote pode ser repassado
review-for-this-machine = para a máquina que o montou
review-rejected = Rejeitadas:

picture-alt = um papel de parede candidato, de { $author }
picture-contrast = contraste
